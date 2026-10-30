# 用例

每个用例都是可运行的完整代码。默认已 `use futures::prelude::*;`。

---

## 用例 1：局域网投屏（DLNA Play-To）

把本机的视频推给局域网里的 DLNA 渲染器（电视 / 盒子 / 音箱）。

### 1.1 整体流程

```
┌──────────┐   ①SSDP 搜索    ┌──────────────────┐
│  你的程序 │ ──────────────► │  渲染器 (DMR)     │
│ (控制点)  │ ◄────────────── │  AVTransport:1    │
└──────────┘   ②拉取描述      └──────────────────┘
     │                              ▲
     │ ③SetAVTransportURI + Play    │ ⑤HTTP GET (Range)
     │──────────────────────────────►│
     │                              │ ⑥拉流播放
     │ ④GET 你的 MP4 服务器 ◄────────┘
```

**控制面（①②③）由本库完成；媒体面（⑤⑥）由你自己的 HTTP 服务器完成。**

### 1.2 控制端代码

```rust
use rupnp::avtransport::Cast;
use rupnp::didl::{class, Didl, DidlItem, DidlResource};
use rupnp::ssdp::{SearchTarget, URN};
use std::time::Duration;

const AV_TRANSPORT: URN = URN::service("schemas-upnp-org", "AVTransport", 1);

#[tokio::main]
async fn main() -> Result<(), rupnp::Error> {
    // 媒体地址必须能被渲染器访问到
    let media_url = "http://192.168.1.10:8080/movie.mp4";

    // ① 搜索带 AVTransport 的设备
    //    SearchTarget::URN(...) 是函数调用，会产生临时值，必须先绑定到变量
    let search_target = SearchTarget::URN(AV_TRANSPORT);
    let devices = rupnp::discover(&search_target, Duration::from_secs(3), None).await?;
    let mut devices = std::pin::pin!(devices);

    while let Some(device) = devices.try_next().await? {
        let Some(service) = device.find_service(&AV_TRANSPORT) else {
            continue;
        };

        println!("投屏到 {}", device.friendly_name());

        let cast = Cast::new(service.clone(), device.url().clone());

        // ② 构造 DIDL-Lite 元数据（决定电视上显示的标题）
        let didl = Didl::new()
            .item(
                DidlItem::new()
                    .title("我的电影")
                    .class(class::VIDEO)
                    .resource(DidlResource::new("http-get:*:video/mp4:*", media_url)),
            )
            .to_xml();

        // ③ 下发推流并播放
        cast.set_uri(media_url, Some(&didl)).await?;
        cast.play().await?;

        // ④ 跟踪播放进度
        loop {
            let info = cast.position_info().await?;
            println!("  track {}  {}/{}", info.track, info.rel_time, info.track_duration);

            if !cast.state().await?.is_playing() {
                break;
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    Ok(())
}
```

### 1.3 媒体服务器：必须支持 HTTP Range

**这是投屏最常见的失败点。** DLNA 渲染器的标准行为是：

```
HEAD /movie.mp4                    → 探测 Content-Length / Accept-Ranges
GET  /movie.mp4  Range: bytes=0-   → 206 Partial Content，开始拉流
GET  /movie.mp4  Range: bytes=N-   → 206，seek 时用
```

不支持 Range 的服务器在渲染器上的表现通常是：状态卡在 `TRANSITIONING` 不动、
播几秒后停、或者 `CurrentTransportStatus` 变成 `ERROR_OCCURRED`。

一个够用的 Range 服务器（放到你自己的 crate 里）：

```rust
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use tokio::net::TcpListener;

/// 在 `port` 上提供 `data`，支持 Range 请求。
async fn serve(data: Bytes, port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;

    loop {
        let (stream, _) = listener.accept().await?;
        let data = data.clone();

        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |req: Request<Incoming>| {
                let data = data.clone();
                async move { Ok::<_, Infallible>(respond(req, &data)) }
            });

            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                eprintln!("connection error: {err}");
            }
        });
    }
}

fn respond(req: Request<Incoming>, data: &Bytes) -> Response<Full<Bytes>> {
    let total = data.len() as u64;

    let builder = Response::builder()
        .header(CONTENT_TYPE, "video/mp4")
        .header(ACCEPT_RANGES, "bytes");

    // 渲染器一般先发 HEAD 探测文件
    if req.method() == Method::HEAD {
        return builder
            .status(StatusCode::OK)
            .header(CONTENT_LENGTH, total)
            .body(Full::new(Bytes::new()))
            .expect("infallible");
    }

    let range = req
        .headers()
        .get(RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_range);

    match range {
        Some((start, end)) if start < total => {
            let end = end.min(total - 1);
            let slice = data.slice(start as usize..=end as usize);

            builder
                .status(StatusCode::PARTIAL_CONTENT)
                .header(CONTENT_RANGE, format!("bytes {start}-{end}/{total}"))
                .header(CONTENT_LENGTH, slice.len())
                .body(Full::new(slice))
                .expect("infallible")
        }
        _ => builder
            .status(StatusCode::OK)
            .header(CONTENT_LENGTH, total)
            .body(Full::new(data.clone()))
            .expect("infallible"),
    }
}

/// 解析 `bytes=start-end` 范围头。
fn parse_range(value: &str) -> Option<(u64, u64)> {
    let spec = value.strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;

    let start: u64 = start.trim().parse().ok()?;
    let end = match end.trim() {
        "" => u64::MAX,
        end => end.parse().ok()?,
    };

    Some((start, end))
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let data = Bytes::from(std::fs::read("movie.mp4")?);
    serve(data, 8080).await
}
```

需要在你自己的 `Cargo.toml` 里加：

```toml
[dependencies]
hyper = { version = "1", features = ["server", "http1"] }
hyper-util = { version = "0.1", features = ["tokio", "server", "http1"] }
http-body-util = "0.1"
bytes = "1"
tokio = { version = "1", features = ["rt-multi-thread", "net", "fs"] }
```

### 1.4 媒体服务器的其它要求

| 要求 | 说明 |
|---|---|
| 监听 `0.0.0.0` | 绑 `127.0.0.1` 设备访问不到 |
| 地址用本机局域网 IP | `http://192.168.1.10:8080/movie.mp4`，不能用 `localhost` |
| `Content-Type` | `video/mp4`，要和 DIDL 里的 `protocolInfo` 对得上 |
| `Accept-Ranges: bytes` | 明确告诉设备支持 Range |
| `HEAD` 支持 | 很多设备先探测 |
| 文件可解码 | H.264 / AAC 这类常规 MP4 没问题 |

### 1.5 排查

| 现象 | 原因 |
|---|---|
| `SetAVTransportURI` 返回 400 / 501 | 参数顺序不对，或 DIDL-Lite 不合法 → 用 `service.scpd(url)` 核对参数顺序 |
| 状态卡在 `TRANSITIONING` | 媒体服务器不可达，或不支持 Range → 看服务器日志有没有收到请求 |
| `CurrentTransportStatus` = `ERROR_OCCURRED` | 设备拉流失败 → 多半是格式不支持或服务器返回了非 2xx |
| `find_service` 返回 `None` | 设备没有 AVTransport 服务，或描述损坏（会被跳过并打 warn 日志） |
| 电视上不显示标题 | DIDL-Lite 没传，或 `dc:title` 命名空间没声明 |
| 事件收不到 | 回调 IP 选错（多网卡）→ 用 `subscribe_with_callback` 显式指定 |

> **同一个服务器，有的 MP4 能播有的不能** —— 大概率是 MP4 的 `moov` 位置问题。
> 完整的分层排查流程见 [常见问题排查](#常见问题排查)。

---

## 用例 2：设备状态查询与音量控制

```rust
use rupnp::ssdp::{SearchTarget, URN};

const RENDERING_CONTROL: URN = URN::service("schemas-upnp-org", "RenderingControl", 1);

#[tokio::main]
async fn main() -> Result<(), rupnp::Error> {
    let search_target = SearchTarget::URN(RENDERING_CONTROL);
    let devices = rupnp::discover(&search_target, Duration::from_secs(3), None).await?;
    let mut devices = std::pin::pin!(devices);

    while let Some(device) = devices.try_next().await? {
        let service = device.find_service(&RENDERING_CONTROL).unwrap();

        // 读当前音量
        let args = "<InstanceID>0</InstanceID><Channel>Master</Channel>";
        let response = service.action(device.url(), "GetVolume", args).await?;
        let volume: u16 = response["CurrentVolume"].parse().unwrap_or(0);
        println!("当前音量 {volume}");

        // 调音量
        let args = "<InstanceID>0</InstanceID><Channel>Master</Channel><DesiredVolume>30</DesiredVolume>";
        service.action(device.url(), "SetVolume", args).await?;
    }

    Ok(())
}
```

### 读 SCPD 确认参数名与顺序

不同厂商的参数名可能不同，先读服务描述最保险：

```rust
let scpd = service.scpd(device.url()).await?;

for action in scpd.actions() {
    if action.name() == "GetVolume" {
        for arg in action.output_arguments() {
            let state_var = arg.related_state_variable();
            println!(
                "{} ({})",
                arg.name(),
                match state_var.kind() {
                    rupnp::scpd::StateVariableKind::Simple(t) => format!("{t:?}"),
                    rupnp::scpd::StateVariableKind::Enum(v) => format!("{v:?}"),
                    rupnp::scpd::StateVariableKind::Range(r) => format!("{r}"),
                }
            );
        }
    }
}
```

---

## 用例 3：事件订阅与状态同步

```rust
use futures::prelude::*;
use rupnp::ssdp::URN;
use std::time::Duration;

const AV_TRANSPORT: URN = URN::service("schemas-upnp-org", "AVTransport", 1);

#[tokio::main]
async fn main() -> Result<(), rupnp::Error> {
    let device = /* 发现到的设备 */;
    let service = device.find_service(&AV_TRANSPORT).unwrap();

    if !service.supports_events() {
        println!("该服务不支持事件订阅");
        return Ok(());
    }

    // 订阅 300 秒
    let (sid, mut stream) = service.subscribe(device.url(), 300).await?;

    // 收到事件时续订，避免订阅过期
    while let Some(state_vars) = stream.try_next().await? {
        for (key, value) in state_vars {
            println!("{key} => {value}");
        }

        service.renew_subscription(device.url(), &sid, 300).await?;
    }

    Ok(())
}
```

### 事件里拿到的是什么

事件是 `<e:propertyset>`，本库把它解析成 `HashMap<String, String>`。
键是状态变量名，值是字符串。AVTransport 的 `LastChange` 变量值本身又是一段
XML，形如：

```xml
<Event xmlns="urn:schemas-upnp-org:metadata-1-0:AVT/">
  <InstanceID val="0">
    <TransportState val="PLAYING"/>
  </InstanceID>
</Event>
```

需要更细的结构就自己用 `rupnp::xml` / `roxmltree` 二次解析。

### 多网卡环境

设备靠反向回调送事件，回调 IP 必须设备能访问到。自动选择会优先挑同网段的网卡，
但 VPN / 虚拟网卡多的机器上仍可能选错：

```rust
use std::net::IpAddr;

let callback_ip: IpAddr = "192.168.1.10".parse().unwrap();
let (sid, mut stream) = service
    .subscribe_with_callback(device.url(), callback_ip, 300)
    .await?;
```

---

## 用例 4：调用自定义 action

有些设备提供厂商自定义的服务，参数在标准之外。做法一样，只是参数要自己拼：

```rust
let service = device
    .find_service(&URN::service("schemas-xxx-com", "Custom", 1))
    .unwrap();

// 先读 SCPD 搞清楚参数
let scpd = service.scpd(device.url()).await?;
for action in scpd.actions() {
    println!("{}: {} 个入参", action.name(), action.input_arguments().count());
}

// 拼参数（记得转义）
let payload = format!(
    "<Param>{}</Param>",
    rupnp::xml::escape_text("value & more")
);
let response = service.action_ordered(device.url(), "CustomAction", &payload).await?;

for (name, value) in response.iter() {
    println!("{name} = {value}");
}
```

---

## 用例 5：批量发现与设备筛选

```rust
use futures::prelude::*;
use rupnp::ssdp::{SearchTarget, URN};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), rupnp::Error> {
    let devices =
        rupnp::discover(&SearchTarget::RootDevice, Duration::from_secs(5), None).await?;
    let mut devices = std::pin::pin!(devices);

    // 设备描述是并发拉取的，产出的顺序与 SSDP 响应顺序一致
    while let Some(device) = devices.try_next().await? {
        // 只要带 AVTransport 的
        if device.find_service(&URN::service("schemas-upnp-org", "AVTransport", 1)).is_none() {
            continue;
        }

        println!(
            "{} [{}] {}",
            device.friendly_name(),
            device.device_type().typ(),
            device.url(),
        );

        // 列出它提供的所有服务
        for service in device.services_iter() {
            println!("    {}", service.service_type().typ());
        }
    }

    Ok(())
}
```

### 读取厂商等扩展字段

```rust
let devices = rupnp::discover_with_properties(
    &SearchTarget::RootDevice,
    Duration::from_secs(5),
    None,
    &["manufacturer", "manufacturerURL", "modelName", "modelNumber"],
).await?;

let mut devices = std::pin::pin!(devices);
while let Some(device) = devices.try_next().await? {
    println!(
        "{} by {} ({})",
        device.friendly_name(),
        device.get_extra_property("manufacturer").unwrap_or("?"),
        device.get_extra_property("modelName").unwrap_or("?"),
    );
}
```

### 导出 SCPD 做离线分析

仓库里的 `examples/dump_scpd.rs` 会把局域网里所有设备的 SCPD 导出到
`descriptions/` 目录，方便对照不同厂商的实现差异。

---

## 用例 6：投文档（PPT / Word）

### 6.1 为什么不能直接投

**DLNA DMR（MediaRenderer）的媒体类别只有三类：image / video / audio。**

UPnP ContentDirectory 虽然定义了 `object.item.textItem`，但 DLNA 认证的渲染器
不包含文字渲染能力，`application/vnd.ms-powerpoint` 这类 mime type 也不在
渲染器的支持列表里。所以**投文档必须先转换**。

关键在于转换发生在**你的服务器端**，UPnP 层面始终只是「推一个媒体 URL」：

```
Word/PPT ──(转换)──> JPEG / MP4 ──(HTTP + Range)──> 渲染器
                        ↑
                   你的转换服务（不在本库范围内）
```

所以本库**一行代码都不用改**，只要把 `upnp:class` 和 `protocolInfo` 换掉。

### 6.2 路线 A：转成图片（兼容性最好）

准备阶段（在你的服务器上做）：

```bash
# pptx → pdf（LibreOffice）
soffice --headless --convert-to pdf slides.pptx --outdir out/

# pdf → 每页一张 JPEG（pdftoppm 来自 poppler-utils）
pdftoppm -jpeg -r 150 out/slides.pdf out/slide
# 得到 out/slide-01.jpg, out/slide-02.jpg, ...
```

投屏端：

```rust
use futures::prelude::*;
use rupnp::avtransport::Cast;
use rupnp::didl::{class, Didl, DidlItem, DidlResource};
use rupnp::ssdp::{SearchTarget, URN};
use std::time::Duration;

const AV_TRANSPORT: URN = URN::service("schemas-upnp-org", "AVTransport", 1);

/// 一页幻灯片
struct Slide {
    title: String,
    url: String,
}

#[tokio::main]
async fn main() -> Result<(), rupnp::Error> {
    // 预先转好图，这里只给出 URL
    let slides = vec![
        Slide {
            title: "封面".to_string(),
            url: "http://192.168.1.10:8080/slides/slide-01.jpg".to_string(),
        },
        Slide {
            title: "第二页".to_string(),
            url: "http://192.168.1.10:8080/slides/slide-02.jpg".to_string(),
        },
    ];

    let search_target = SearchTarget::URN(AV_TRANSPORT);
    let devices = rupnp::discover(&search_target, Duration::from_secs(3), None).await?;
    let mut devices = std::pin::pin!(devices);

    let Some(device) = devices.try_next().await? else {
        eprintln!("没有找到渲染器");
        return Ok(());
    };

    let service = device
        .find_service(&AV_TRANSPORT)
        .expect("设备没有 AVTransport");

    let cast = Cast::new(service.clone(), device.url().clone());

    for slide in &slides {
        let didl = Didl::new()
            .item(
                DidlItem::new()
                    .title(slide.title.clone())
                    .class(class::IMAGE) // object.item.imageItem.photo
                    .resource(DidlResource::new(
                        "http-get:*:image/jpeg:*",
                        &slide.url,
                    )),
            )
            .to_xml();

        // 翻页 = 换 URI 再播
        cast.set_uri(&slide.url, Some(&didl)).await?;
        cast.play().await?;

        // 这一页停留多久
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    Ok(())
}
```

要点：

- `class::IMAGE` = `object.item.imageItem.photo`
- `protocolInfo` 用 `http-get:*:image/jpeg:*`（PNG 则是 `image/png`）
- **图片同样需要支持 Range**，很多渲染器拉图片时也发 Range 请求

> 图片投屏失败时，同样按 [常见问题排查](#常见问题排查) 的流程走。

### 6.3 路线 B：转成视频（体验最好）

唯一能保留翻页动画的方案 —— 翻页变成视频里的转场，**零延迟**。

```bash
# 静态版式：把每页图片合成一个视频（每页停 5 秒）
ffmpeg -framerate 1/5 -i out/slide-%02d.jpg \
       -c:v libx264 -pix_fmt yuv420p -vf "scale=1920:-2" slides.mp4
```

投屏端只要把 `class` 和 `protocolInfo` 换成视频的：

```rust
let didl = Didl::new()
    .item(
        DidlItem::new()
            .title("我的幻灯片")
            .class(class::VIDEO) // object.item.videoItem
            .resource(DidlResource::new(
                "http-get:*:video/mp4:*",
                "http://192.168.1.10:8080/slides.mp4",
            )),
    )
    .to_xml();

cast.set_uri("http://192.168.1.10:8080/slides.mp4", Some(&didl)).await?;
cast.play().await?;
```

> 带动画的 PPT 要保住动效，需要用 PowerPoint 导出或录屏，ffmpeg 合成只能保留
> 静态版式。

### 6.4 路线 C：厂商私有扩展（不通用）

三星、LG 等电视的 DLNA 服务器端可以推送 `.doc` / `.ppt` / `.xls`，由电视内置的
Office 渲染器显示。但这是**厂商私有协议，不是 UPnP 标准**：

- 不同品牌互不通用，同品牌不同型号也可能不同
- 需要往 DIDL-Lite 里塞私有标签，本库的 `DidlElement::new(Namespace::Dlna, …)`
  可以做到，但具体标签名得对着那台电视的文档来
- 走的是 ContentDirectory（电视当 DMS）而不是 AVTransport

**不建议作为产品能力。**

### 6.5 翻页延迟

路线 A 逐页 `SetAVTransportURI` + `Play`，每次翻页都要重新走一遍
「加载 → 缓冲 → 播放」，延迟通常几百毫秒到 2 秒，翻 PPT 会明显卡。

有些渲染器的 AVTransport 提供了 `SetAVTransportURIList`：

```
SetAVTransportURIList(InstanceID, CurrentIndex, CurrentURIList)
```

一次把多页推给设备，让它按列表播，能显著降低翻页延迟。

**但 `CurrentURIList` 的格式各设备不一致**（有的是逗号分隔的 URI 串，
有的是转义后的 DIDL-Lite 串），需要对着具体设备试。手工拼的话：

```rust
/// 一次推送多页。
///
/// `CurrentURIList` 的格式各设备不一致：有的接受逗号分隔的 URI，有的接受
/// 转义后的 DIDL-Lite 串，需要对着具体设备试。
fn set_uri_list_payload(instance_id: u32, current_index: u32, uris: &[&str]) -> String {
    format!(
        "<InstanceID>{instance_id}</InstanceID>\
         <CurrentIndex>{current_index}</CurrentIndex>\
         <CurrentURIList>{uris}</CurrentURIList>",
        uris = rupnp::xml::escape_text(&uris.join(",")),
    )
}

// 用法
let urls = vec![
    "http://192.168.1.10:8080/slides/slide-01.jpg",
    "http://192.168.1.10:8080/slides/slide-02.jpg",
];
let payload = set_uri_list_payload(0, 0, &urls);
service
    .action(device.url(), "SetAVTransportURIList", &payload)
    .await?;
```

先用设备的 SCPD 确认这个 action 存在：

```rust
let scpd = service.scpd(device.url()).await?;
let has_list = scpd
    .actions()
    .iter()
    .any(|a| a.name() == "SetAVTransportURIList");
```

彻底消除延迟还是走路线 B（转视频）。

---

## 常见问题排查

投屏失败的根因分布在三个层次，**按顺序排查**能最快定位。

```
① HTTP 层    服务器的 Range 响应合规吗？
       ↓ 是
② 容器层    MP4 的 moov 在 mdat 之前吗？      ← 90% 的情况栽在这
       ↓ 是
③ 编码层    渲染器解得了这个编码吗？
       ↓ 是
④ 网络层    渲染器和服务器之间有没有中间设备捣乱？
```

---

### 第 1 步：HTTP 层

渲染器拉流的标准请求序列是：

```
HEAD /movie.mp4                    → 拿 Content-Length
GET  /movie.mp4  Range: bytes=0-   → 206 Partial Content
GET  /movie.mp4  Range: bytes=N-   → 206，seek 时用
```

**响应只要有一处不合规，有些渲染器会直接放弃**：

| 问题 | 症状 |
|---|---|
| 不支持 Range，返回 200 + 全量 | 每次从头拉，或者直接报错 |
| 返回 200 而不是 **206** | 渲染器可能当成错误 |
| 缺 `Accept-Ranges: bytes` | 渲染器不知道能 seek，直接放弃 |
| `Content-Range` 格式不对 | 解析失败 |
| 只支持单区间，不支持多区间 `bytes=0-100,200-300` | 有的渲染器会发多区间请求 |
| `Content-Type` 不是 `video/mp4`（比如 `application/octet-stream`） | 有的渲染器按 mime 判断能力 |
| HEAD 没实现，或返回的 `Content-Length` 与 GET 不一致 | 渲染器认为文件变了 |

验证（这两条必须过）：

```bash
curl -I http://your-server/movie.mp4
# 需要看到：200 OK / Accept-Ranges: bytes / Content-Length / Content-Type: video/mp4

curl -r 0-1023 -o /dev/null -D - http://your-server/movie.mp4
# 需要看到：206 Partial Content / Content-Range: bytes 0-1023/<总长> / Content-Length: 1024
```

---

### 第 2 步：容器层 —— moov 在哪（最常见）

MP4 的顶层结构是若干 atom：

```
[ftyp] [moov] [mdat]     ← moov 在前（fast start 已优化）
[ftyp] [mdat] [moov]     ← moov 在后（未优化）
```

**`moov` 里装着播放必需的信息**：时长、轨道、编码参数、sample 索引
（`stco`/`co64`）、关键帧表（`stss`）。

- **moov 在前** → 渲染器第一次 `Range: bytes=0-` 就拿到 moov，立刻知道文件多长、
  编码是什么、关键帧在哪 → **秒开** ✓
- **moov 在后** → 渲染器第一次只拿到 ftyp 和一堆 mdat 数据，**它不知道文件多长、
  也不知道编码是什么**，必须把整个文件下载完才能读到末尾的 moov →
  2GB 的文件就要等下载完 2GB 才能播 → **表现为一直转圈或直接超时失败**

诊断工具（仓库里的 `mp4-tool`，纯 Rust 实现，在 `src/bin/` 下）：

```bash
cargo run --bin mp4-tool -- check movie.mp4
```

```
=== movie.mp4 ===
          偏移  atom               大小
----------------------------------------
           0  ftyp               24
          24  moov             1234
        1258  mdat        2097152

结论: moov 在 mdat 之前 —— fast start 已优化，可以秒开
```

退出码 0 表示布局良好，1 表示需要重新封装（可以拿来做批量检查）。
`check` 只需要读文件，不需要 ffmpeg。

**修复**：

```bash
cargo run --bin mp4-tool -- fix movie.mp4
```

它会调 ffmpeg 做 `-c copy -movflags +faststart`，**不重编码、画质无损**，
结果写到 `movie.faststart.mp4`。

### ffmpeg 从哪来

| 情况 | 行为 |
|---|---|
| 本机已有 ffmpeg | 直接用 |
| `--ffmpeg <路径>` | 用指定的这个，不会下载 |
| 都没指定 | **创建下载目录并下载**一个 |

下载走 Rust 的 HTTP 客户端（`ffmpeg-sidecar` 用的 `ureq`），**不依赖系统里有没有
curl**；下载过程显示进度，失败会自动重试（退避时间逐次翻倍）。

```bash
# 用本机已有的 ffmpeg
cargo run --bin mp4-tool -- fix --ffmpeg /usr/local/bin/ffmpeg movie.mp4

# 指定下载目录
cargo run --bin mp4-tool -- fix --ffmpeg-dir ~/.local/bin movie.mp4

# 国内网络直连境外源很慢时，用镜像
cargo run --bin mp4-tool -- fix --ffmpeg-url \
  https://gh-proxy.com/https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip \
  movie.mp4
```

> 为什么有些文件天然就是 fast start、有些不是？因为**录制顺序不同**。
> 相机、录屏软件边录边写，moov 只能在录制结束时才写到末尾；而经过 ffmpeg
> 或 Web 转码的文件通常已经优化过了。

---

### 第 3 步：编码层

容器和 HTTP 都对，编码不支持照样黑屏：

| 类型 | 支持情况 |
|---|---|
| H.264 (AVC) | ✅ 几乎所有设备 |
| H.265 / HEVC | ⚠️ **很多老电视不支持** |
| MPEG-2 | ✅ 普遍支持（老标准） |
| VP9 / AV1 | ❌ 基本不支持 |
| AAC / MP3 / AC-3 | ✅ |
| Opus / FLAC | ⚠️ 视设备 |
| H.264 10bit | ❌ 多数电视只支持 8bit |
| YUV422p / YUV444p | ❌ 多数电视只支持 YUV420p |

检查：

```bash
ffprobe -v error -select_streams v:0 \
  -show_entries stream=codec_name,profile,level,pix_fmt,width,height \
  -of default=nw=1 input.mp4
```

重点看 `codec_name` 是不是 `h264`、`pix_fmt` 是不是 `yuv420p`。

---

### 第 4 步：网络层

用 `curl -r` 从**渲染器所在的网络**测一遍，而不是从本机测。
企业代理、防火墙、透明网关可能截断或重组 Range 响应。

另外确认渲染器真的能连到你的服务器：

- 服务器在 Docker / 虚拟机里，端口没映射到局域网
- 机器有多张网卡，渲染器连到的不是你以为的那个地址
- 路由器开了 **AP 隔离**（client isolation），客户端之间互不通信

---

### 症状速查

| 症状 | 最可能的原因 |
|---|---|
| 一直转圈，几分钟后突然开始播 | moov 在文件末尾（第 2 步） |
| 立刻报错 / 拒绝播放 | HTTP 响应不合规，或编码不支持 |
| 能播但进度条不动 | `mvhd.duration` 为 0，或缺 `stss` 关键帧表 |
| 能播，拖进度条就跳回开头 | 缺关键帧索引，或 `stco` 偏移量错误 |
| 播到一半断掉 | 文件被截断，或索引指向了文件外 |
| 同一个服务器，有的文件能播有的不能 | moov 位置不同（第 2 步） |
| 提示格式错误 / 不支持的格式 | fMP4（`moof`+`mfra`）老渲染器不认，或编码是 HEVC |
| 偶尔能播偶尔不能 | 网络中间设备干扰（第 4 步） |

---

### 其它值得知道的坑

**碎片化 MP4（fMP4）**
HLS / DASH 直播用的 `moof` + `mfra` 结构，**老渲染器不认**。症状是
"格式错误"而不是"转圈"，和 moov 问题表现完全不同，别搞混。

**索引缺失或文件被截断**
没有 `stss`（关键帧表）→ seek 直接失败；`stco`/`co64` 的偏移量对不上
（文件下载不完整或被改过）→ 播到某处就断。

**能播但不能 seek**
moov 正常但 `stss` 缺失，或关键帧全是 P 帧 —— 表现为能播，拖进度条就卡死
或跳回开头。

---

## 用例 7：媒体处理（分割 / 拼接 / 插入 / 生成空白片）

`rupnp::mp4_tool::media` 把常用的 MP4 / MP3 处理封装好了，命令行在
`src/bin/mp4-tool.rs`，底层 API 在 `src/mp4_tool/`。

时间参数用 ffmpeg 的时长格式：`90`、`1:30`、`00:01:30`、`00:01:30.500` 都支持。

### 7.1 分割

```bash
# 从第 30 秒开始，取 60 秒
cargo run --bin mp4-tool -- split movie.mp4 --start 00:00:30 --duration 60
```

输出 `movie.cut.mp4`。用 `-c copy` 不重编码。

```rust
use std::path::Path;
use rupnp::mp4_tool;

let ffmpeg = mp4_tool::ensure_ffmpeg(&mp4_tool::FfmpegOptions::default())?;
let out = mp4_tool::split(&ffmpeg, Path::new("movie.mp4"), "00:00:30", "60")?;
```

### 7.2 拼接

```bash
# 按命令行顺序拼
cargo run --bin mp4-tool -- concat part1.mp4 part2.mp4 part3.mp4
```

输出 `part1.concat.mp4`。

**要求所有输入是同一种容器和编码** —— `-c copy` 只是把数据搬过去，不做转码。
混了不同分辨率 / 编码的文件会拼接失败，需要先用 ffmpeg 统一转一遍。

```rust
let out = mp4_tool::concat(&ffmpeg, &[
    "part1.mp4".into(),
    "part2.mp4".into(),
])?;
```

### 7.3 插入

```bash
# 在第 5 分钟处插入一段广告
cargo run --bin mp4-tool -- insert movie.mp4 --insert ad.mp4 --at 00:05:00
```

输出 `movie.inserted.mp4`。

实现是「切成两段 → 拼接」，所以同样要求同一种容器和编码。
中间产物会写到系统临时目录并自动清理。

```rust
let out = mp4_tool::insert(
    &ffmpeg,
    Path::new("movie.mp4"),
    Path::new("ad.mp4"),
    "00:05:00",
)?;
```

### 7.4 生成黑屏视频 / 静音音频

```bash
# 4 秒黑屏视频（1280x720，带静音音轨）
cargo run --bin mp4-tool -- blank --kind video --duration 4

# 2 秒静音音频
cargo run --bin mp4-tool -- blank --kind audio --duration 2
```

输出到系统临时目录。用途：

- 给拼接的片段之间加空隙
- 给投屏做「占位画面」
- 测试渲染器在无内容时的表现

```rust
use rupnp::mp4_tool::media::{self, MediaKind};

let out = media::blank(&ffmpeg, MediaKind::Video, "4")?;
```

### 7.5 输出与中间文件

| 子命令 | 输出位置 |
|---|---|
| `split` | `<名字>.cut.<扩展名>`，与输入同目录 |
| `concat` | `<第一个输入名字>.concat.<扩展名>`，与输入同目录 |
| `insert` | `<名字>.inserted.<扩展名>`，与输入同目录 |
| `blank` | 系统临时目录 `rupnp-blank-<pid>-<nanos>.<扩展名>` |
| `convert` | `<名字>.converted.mp4`，与输入同目录 |

`insert` 切出来的两段放在系统临时目录，命令结束后自动删除。

> 所有操作都**不会覆盖输入文件**。同一份输入多次执行 `split` 会覆盖上一次的
> `.cut.` 输出。

---

## 用例 8：通用格式转 MP4

把 AVI / FLV / WMV / MOV / MKV 等转成 MP4。和 `split` / `concat` 不同，
这里输入容器和编码都未知，必要时必须重编码。

```bash
# 单个文件
cargo run --bin mp4-tool -- convert movie.avi

# 批量，画质优先
cargo run --bin mp4-tool -- convert a.flv b.wmv c.mkv --quality high

# 压体积：等比缩到 720p 以内
cargo run --bin mp4-tool -- convert movie.mkv --max-height 720
```

输出 `<名字>.converted.mp4`。**批量时单个文件失败不会中断整批**，最后汇总失败数。

### 8.1 性能：先探测，能 copy 就不转

转码是整条链路上最贵的一步。这个实现会先跑一次 `ffmpeg -i` 探测容器和编码，
再决定要不要重编码：

| 输入情况 | 做法 | 耗时相对 |
|---|---|---|
| MP4 + H.264 + AAC | **只换容器**（`-c copy` + faststart） | 秒级，画质零损失 |
| 视频已是 H.264，音频不是 | 只重编码音频 | 很省 |
| 视频不是 H.264（WMV/FLV/HEVC…） | 重编码视频，音频能 copy 就 copy | 全量 |

实测三种码流组合的输出：

```
avi/mpeg4/mp3   -> h264/aac   重编码视频+音频
mkv/hevc/ac3    -> h264/aac   重编码视频+音频
mp4/h264/mp3    -> h264/aac   只重编码音频
mp4/h264/aac    -> h264/aac   只换容器，未重编码
```

### 8.2 画质档位

| 档位 | preset | CRF | 音频 | 适用 |
|---|---|---|---|---|
| `--quality fast` | `ultrafast` | 28 | 128k | 批量预处理，速度优先 |
| 默认（balanced） | `veryfast` | 23 | 128k | 一般用途 |
| `--quality high` | `slow` | 20 | 192k | 最终交付 |

也可以直接指定 `--preset` / `--crf` / `--max-height` 覆盖。

`preset` 对速度的影响很大：`ultrafast` 比 `veryslow` 快 10～20 倍，体积则大
2～3 倍。批量转码一律先用 `fast`，确认没问题再对最终产物重跑 `high`。

### 8.3 输出参数

默认偏向「能被 DLNA 渲染器直接播放」：

- 视频 `libx264` + `yuv420p`（兼容性最好，10bit / 444 很多电视解不了）
- 音频 `aac` `128k`
- `-movflags +faststart`（moov 前置，投屏必需，见
  [常见问题排查](#常见问题排查)）

这些都可以在 `TranscodeOptions` 里改。

### 8.4 用库

```rust
use std::path::Path;
use rupnp::mp4_tool::{self, TranscodeOptions};

let ffmpeg = mp4_tool::ensure_ffmpeg(&mp4_tool::FfmpegOptions::default())?;

// 先看看是什么格式，决定要不要转
let info = mp4_tool::probe(&ffmpeg, Path::new("movie.mkv"))?;
println!("{} / {:?} / {:?}", info.format, info.video_codec, info.audio_codec);

// 转码
let outcome = mp4_tool::to_mp4(
    &ffmpeg,
    Path::new("movie.mkv"),
    Path::new("movie.converted.mp4"),
    &TranscodeOptions::high_quality(),
)?;

println!(
    "remuxed={} 视频重编码={} 音频重编码={}",
    outcome.remuxed, outcome.reencoded_video, outcome.reencoded_audio
);

// 批量
let results = mp4_tool::to_mp4_batch(
    &ffmpeg,
    &["a.avi".into(), "b.flv".into()],
    &TranscodeOptions::default(),
);
```

### 8.5 转码为什么慢

`-c copy` 只搬字节；一旦重编码，CPU 就要逐帧解码 + 编码。影响速度的因素按重要性排序：

1. **分辨率** —— 1080p 比 720p 慢 2～3 倍
2. **preset** —— 见上表
3. **是否用硬件编码** —— `h264_nvenc` / `h264_qsv` / `h264_videotoolbox` 能快 3～10 倍，
   但画质同码率下略差，且需要显卡支持

需要硬件编码的场景，目前直接用 `--preset` 搭配 FFmpegCommand 还不够（本模块没暴露
编码器选择）。可以先用本模块的 `probe` 判断格式，再自己拼 ffmpeg 命令。

---

## 用例 9：音频提取与转换（MP3）

两个操作，都不碰视频流，所以很快：

- **`extract-mp3`** —— 从视频里把音轨抽出来
- **`to-mp3`** —— 把音频（WAV / FLAC / M4A …）转成 MP3

```bash
# 从视频提取音轨
cargo run --bin mp4-tool -- extract-mp3 movie.mp4

# WAV 转 MP3
cargo run --bin mp4-tool -- to-mp3 song.wav

# 高码率
cargo run --bin mp4-tool -- to-mp3 song.wav --bitrate 320k

# 语音 / 播客：单声道 64k
cargo run --bin mp4-tool -- to-mp3 talk.wav --bitrate 64k --channels 1 --sample-rate 44100
```

输出 `<名字>.mp3`，与输入同目录。支持一次传多个文件，单个失败不影响其它，
最后汇总失败数。

### 9.1 参数

| 选项 | 说明 |
|---|---|
| `-b, --bitrate <码率>` | 默认 `192k`；`320k` 接近无损，`64k` 适合语音 |
| `--sample-rate <Hz>` | 采样率，例如 `44100`；不指定则保持原样 |
| `--channels <数量>` | `1` 单声道、`2` 立体声；不指定则保持原样 |

`extract-mp3` 会把视频流丢掉（`-vn`），只取**第一条音轨**（`-map 0:a:0`）。

### 9.2 没有音轨时会明确报错

```console
$ mp4-tool extract-mp3 noaudio.mp4
提取 noaudio.mp4 的音轨失败: noaudio.mp4 里没有音轨
1 个文件失败
```

退出码非 0，且不会留下半成品文件。这个检查是在调用 ffmpeg 之前用 `ffmpeg -i`
的输出判断的，比等 ffmpeg 报「Stream map '0:a:0' matches no streams」更清楚。

### 9.3 用库

```rust
use std::path::Path;
use rupnp::mp4_tool::{self, Mp3Options};

let ffmpeg = mp4_tool::ensure_ffmpeg(&mp4_tool::FfmpegOptions::default())?;

// 从视频提取，默认 192k
let mp3 = mp4_tool::extract_mp3(&ffmpeg, Path::new("movie.mp4"), None, &Mp3Options::default())?;
println!("{}", mp3.display());

// WAV 转 MP3，高码率
let mp3 = mp4_tool::to_mp3(&ffmpeg, Path::new("song.wav"), None, &Mp3Options::high())?;

// 也可以指定输出路径
let mp3 = mp4_tool::to_mp3(
    &ffmpeg,
    Path::new("song.wav"),
    Some(Path::new("/data/out.mp3")),
    &Mp3Options::voice(),
)?;
```

`Mp3Options` 提供三个预设：

| 预设 | 码率 | 采样率 | 声道 | 用途 |
|---|---|---|---|---|
| `default()` | 192k | 原样 | 原样 | 一般用途 |
| `high()` | 320k | 原样 | 原样 | 接近无损 |
| `voice()` | 64k | 44100 | 单声道 | 语音、播客、有声书 |

批量：

```rust
let results = mp4_tool::to_mp3_batch(
    &ffmpeg,
    &["a.wav".into(), "b.wav".into()],
    &Mp3Options::default(),
);
```

### 9.4 为什么比转视频快

只处理音频，没有视频解码和编码。一个 1 小时的视频，提取音轨通常十几秒到一分钟；
转码成 MP4 则要几分钟到几十分钟。

如果需要**无损**保存音轨，把 `--bitrate` 换成直接 `-c:a copy` 的路径（本模块目前
只输出 MP3）。WAV 转 MP3 本身就是有损压缩，`320k` 已经接近人耳极限。

---

## 用例 10：MP3 拼接（间隔 + 淡入淡出）

把多个音频接成一个，可以：

- 在段与段之间留一段**静音间隔**（像节目单那样一段段接）
- 每段首尾加**淡入淡出**，听感更自然

```bash
# 无缝拼接
cargo run --bin mp4-tool -- concat-mp3 a.mp3 b.mp3 c.mp3

# 每段之间静音 1.5 秒，每段首尾 0.5 秒淡入淡出
cargo run --bin mp4-tool -- concat-mp3 a.mp3 b.mp3 c.mp3 --gap 1.5 --fade 0.5

# 指定输出路径和曲线
cargo run --bin mp4-tool -- concat-mp3 a.mp3 b.mp3 \
  --gap 2 --fade 1 --curve linear -o playlist.mp3
```

输出默认 `<第一个输入名字>.concat.mp3`，与输入同目录。

### 10.1 选项

| 选项 | 说明 |
|---|---|
| `--gap <秒>` | 段与段之间的静音间隔。默认 0（无缝） |
| `--fade <秒>` | 每段首尾的淡入淡出时长。默认 0（不淡） |
| `--curve <形状>` | `linear` / `sine` / `exp`，默认 `sine` |
| `-o, --output <路径>` | 输出路径 |
| `-b, --bitrate <码率>` | 输出 MP3 的码率，默认 `192k` |

### 10.2 三种曲线

| 曲线 | ffmpeg | 听感 |
|---|---|---|
| `linear` | `tri` | 最平，音量线性变化 |
| `sine`（默认） | `qsin` | 起伏自然，适合背景音乐 |
| `exp` | `exp` | 变化剧烈，适合强调转场 |

### 10.3 实现：两遍

1. **每段单独处理** —— 需要淡入淡出就重编码（加 `afade` 滤镜），不需要就
   `-c copy` 直接复制；统一采样率和声道
2. **再拼接** —— 用 concat demuxer 把处理好的片段和间隔静音拼起来

没有一趟 `filter_complex` 做完，是因为：淡出的**起点需要每段的时长**，而间隔静音的
采样率/声道必须和音轨一致。探测出来更稳妥，代价是多一遍临时文件。

中间产物放在系统临时目录，命令结束后自动删除。

### 10.4 淡入淡出会被限制在片段时长的一半以内

如果某段只有 1 秒，却要求 2 秒的淡入淡出，整段会变成静音。所以实际生效的淡入
淡出时长会被压到 `min(fade, 时长/2)`。

探测不到时长时（比如文件损坏）就不做淡入淡出，而不是猜一个。

### 10.5 实测

三段音频（3s / 2s / 4s）：

```
无缝拼接        9.07s   期望 ~9.0
间隔1.5+淡入淡出 12.15s  期望 ~12.0
```

用 `astats` 逐点测 RMS，确认淡入淡出和间隔真的作用在音频上：

```
0.05s = -31.5dB   1.5s = -22.4dB    起始更轻 -> 淡入生效
3.15s = -inf      1.5s = -22.4dB    间隔是纯静音
11.95s = -36.0dB  11.0s = -22.4dB   结尾更轻 -> 淡出生效
```

### 10.6 用库

```rust
use std::path::{Path, PathBuf};
use std::time::Duration;
use rupnp::mp4_tool::{self, ConcatMp3Options, FadeCurve};

let ffmpeg = mp4_tool::ensure_ffmpeg(&Default::default())?;

let options = ConcatMp3Options {
    gap: Duration::from_secs(1),
    fade: Duration::from_millis(500),
    curve: FadeCurve::Sine,
    ..Default::default()
};

mp4_tool::concat_mp3(
    &ffmpeg,
    &[PathBuf::from("a.mp3"), PathBuf::from("b.mp3")],
    Path::new("playlist.mp3"),
    &options,
)?;
```

预设：

```rust
// 无缝
ConcatMp3Options::seamless()

// 带间隔和淡入淡出
ConcatMp3Options::with_gap_and_fade(Duration::from_secs(1), Duration::from_millis(500))
```

---

## 用例 11：音频分割（起止时间）

截取音频的 `[开始, 结束)` 这一段，存成 MP3。

```bash
# 取第 30 秒到第 90 秒
cargo run --bin mp4-tool -- split-audio song.mp3 --start 00:00:30 --end 00:01:30

# 指定输出
cargo run --bin mp4-tool -- split-audio song.mp3 --start 1:30 --end 2:00 -o intro.mp3
```

输出默认 `<名字>.split.mp3`，与输入同目录。

### 11.1 时间格式

`--start` 和 `--end` 都接受这些写法：

| 写法 | 含义 |
|---|---|
| `90`、`1.5` | 秒 |
| `1:30` | 分:秒 |
| `00:01:30` | 时:分:秒 |
| `00:01:30.500` | 带小数秒 |
| `90s`、`2m`、`1h30m` | 带单位 |

库里的 `parse_time` 也公开了，可以自己用：

```rust
use std::time::Duration;
use rupnp::mp4_tool::parse_time;

assert_eq!(parse_time("1:30"), Some(Duration::from_secs(90)));
assert_eq!(parse_time("1h30m"), Some(Duration::from_secs(5400)));
```

### 11.2 边界检查

- **结束时间必须晚于开始时间** —— 否则报错，不产出文件
- **时间格式非法** —— 报错并说明是哪个时间解析不了

```console
$ mp4-tool split-audio song.mp3 --start 5 --end 2
错误: 结束时间必须晚于开始时间（5s -> 2s）
```

### 11.3 复制还是重编码

默认用 `-c copy` 只复制数据，快且对 MP3 无损。MP3 每帧都能独立解码，所以切点本来就是
准的。

换成 AAC 之类有帧间依赖的格式时，切点可能偏一个帧，这时加 `--reencode` 重新编码：

```bash
cargo run --bin mp4-tool -- split-audio track.m4a --start 30 --end 90 --reencode
```

### 11.4 用库

```rust
use std::path::Path;
use rupnp::mp4_tool::{self, SplitAudioOptions};

let ffmpeg = mp4_tool::ensure_ffmpeg(&Default::default())?;

let out = mp4_tool::split_audio(
    &ffmpeg,
    Path::new("song.mp3"),
    "00:00:30",
    "00:01:30",
    None,
    &SplitAudioOptions::default(),
)?;
println!("{}", out.display());
```
