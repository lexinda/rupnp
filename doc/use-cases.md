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
    let devices =
        rupnp::discover(&SearchTarget::URN(AV_TRANSPORT), Duration::from_secs(3), None).await?;
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
            let slice = data.slice(start..=end);

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

---

## 用例 2：设备状态查询与音量控制

```rust
use rupnp::ssdp::{SearchTarget, URN};

const RENDERING_CONTROL: URN = URN::service("schemas-upnp-org", "RenderingControl", 1);

#[tokio::main]
async fn main() -> Result<(), rupnp::Error> {
    let devices =
        rupnp::discover(&SearchTarget::URN(RENDERING_CONTROL), Duration::from_secs(3), None).await?;
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
    .find_service(&URN::service("urn:schemas-xxx-com:service:Custom:1"))
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
