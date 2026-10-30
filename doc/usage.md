# 使用方法

本文覆盖全部公开 API。示例默认 `use futures::prelude::*;`（为了 `try_next`）与
`use std::time::Duration;`。

---

## 1. 发现设备

```rust
use futures::prelude::*;
use rupnp::ssdp::{SearchTarget, URN};
use std::time::Duration;

// 搜索所有根设备
let devices = rupnp::discover(&SearchTarget::RootDevice, Duration::from_secs(3), None).await?;
let mut devices = std::pin::pin!(devices);

while let Some(device) = devices.try_next().await? {
    println!("{} - {}", device.friendly_name(), device.url());
}
```

`discover` 返回的是流，必须 `pin` 住才能 `try_next`。

### 搜索特定类型的服务

比搜根设备更精准，也更快：

```rust
const AV_TRANSPORT: URN = URN::service("schemas-upnp-org", "AVTransport", 1);

// 注意：SearchTarget::URN(...) 是函数调用，会产生临时值。
// discover 返回的流借用了它，所以必须先绑定到变量，否则编译不过。
let search_target = SearchTarget::URN(AV_TRANSPORT);
let devices = rupnp::discover(&search_target, Duration::from_secs(3), None).await?;
```

> `SearchTarget::RootDevice` 是常量，可以直接写 `&SearchTarget::RootDevice`。

### 参数

```rust
pub async fn discover(
    search_target: &SearchTarget,
    timeout: Duration,      // SSDP 搜索时长
    ttl: Option<u32>,       // 放进 M-SEARCH 的 TTL 字段
) -> Result<impl Stream<Item = Result<Device>>>
```

`ttl` 是 SSDP 报文的 TTL（秒），有些设备会据此决定回复几个包。

### 顺带读取扩展属性

`full_device_spec` feature 只暴露固定的一批字段。任意属性用
`discover_with_properties`：

```rust
let devices = rupnp::discover_with_properties(
    &SearchTarget::RootDevice,
    Duration::from_secs(3),
    None,
    &["manufacturer", "manufacturerURL", "modelName"],
).await?;

let mut devices = std::pin::pin!(devices);
while let Some(device) = devices.try_next().await? {
    println!("{}", device.get_extra_property("manufacturer").unwrap_or_default());
}
```

> 注意 `Device` 通过 `Deref` 指向 `DeviceSpec`，所以 `device.get_extra_property(...)`
> 直接可用。

---

## 2. 已知 URL 直接读设备

SSDP 有时不可用（组播被禁、设备不支持 M-SEARCH），这时直接拿描述 URL：

```rust
use rupnp::{http::Uri, Device};

let url = Uri::from_static("http://192.168.1.5:1400/xml/device_description.xml");
let device = Device::from_url(url.clone()).await?;

println!("{}", device.friendly_name());
```

带超时和扩展属性的变体：

```rust
let device = Device::from_url_with_timeout(url.clone(), Duration::from_secs(5)).await?;
let device = Device::from_url_and_properties(url.clone(), &["manufacturer"]).await?;
let device = Device::from_url_and_properties_with_timeout(
    url,
    &["manufacturer"],
    Duration::from_secs(5),
).await?;
```

> `Uri` 不是 `Copy`，多次调用需要 `url.clone()`。

---

## 3. 设备描述与设备树

`Device` = 描述 URL + 设备描述内容。内容在 `DeviceSpec` 里，`Device` 通过
`Deref` 暴露它。

```rust
device.device_type()        // &URN，设备类型
device.friendly_name()      // &str
device.get_extra_property("manufacturer")   // Option<&str>
```

### 遍历子设备与服务

```rust
// 直接子设备 / 直接服务
device.devices()      // &Vec<DeviceSpec>
device.services()     // &Vec<Service>

// 递归遍历所有层级
device.devices_iter()  // impl Iterator<Item = &DeviceSpec>
device.services_iter() // impl Iterator<Item = &Service>

// 按类型查找
device.find_service(&URN::service("schemas-upnp-org", "AVTransport", 1))
device.find_device(&URN::device("schemas-upnp-org", "MediaRenderer", 1))
```

`find_service` / `find_device` 都是**递归**的，会搜到子设备里的。

### 完整设备字段（`full_device_spec` feature）

启用后额外可用：

```rust
device.manufacturer()        // &str
device.manufacturer_url()    // Option<&str>
device.model_name()          // &str
device.model_description()   // Option<&str>
device.model_number()        // Option<&str>
device.model_url()           // Option<&str>
device.serial_number()       // Option<&str>
device.udn()                 // &str
device.upc()                 // Option<&str>
device.presentation_url()    // Option<&str>
```

---

## 4. 调用 action

所有 action 的调用方式都一样：服务 + 目标 URL + action 名 + 参数片段。

```rust
let service = device.find_service(&AV_TRANSPORT).unwrap();

let args = "<InstanceID>0</InstanceID><CurrentURI>http://h/movie.mp4</CurrentURI>";
let response = service.action(device.url(), "SetAVTransportURI", args).await?;

println!("{:?}", response);
```

返回的是 `HashMap<String, String>`，键是 action 的输出参数名。

### 参数顺序

**参数片段必须按服务描述（SCPD）里 `argumentList` 的顺序拼接**，这是 UPnP 的硬性要求。
可以先用 `scpd()` 读出参数顺序：

```rust
let scpd = service.scpd(device.url()).await?;
for action in scpd.actions() {
    println!("{}", action.name());
    for arg in action.input_arguments() {
        println!("  <- {}", arg.name());
    }
    for arg in action.output_arguments() {
        println!("  -> {}", arg.name());
    }
}
```

### 转义参数值

**payload 是原样拼进 SOAP Body 的，库不替你转义。** 含 `&` `<` `>` 的值必须自己处理：

```rust
use rupnp::xml::escape_text;

let uri = "http://h/movie.mp4?a=1&b=2";
let args = format!(
    "<InstanceID>0</InstanceID><CurrentURI>{}</CurrentURI>",
    escape_text(uri),
);
```

`escape_text` 用于元素内容，`escape_attr` 用于属性值（额外处理引号与空白字符）。

### 类型化、保序的响应

`HashMap` 丢失参数顺序，也没有类型。用 `action_ordered`：

```rust
let response = service.action_ordered(device.url(), "GetPositionInfo", args).await?;

let track: u32 = response.get_u32("Track")?;
let rel_time = response.get("RelTime").unwrap_or_default();
let playing = response.get_bool("Enabled")?;

// 保序遍历
for (name, value) in response.iter() {
    println!("{name} = {value}");
}
```

`ActionResponse` 提供 `get` / `get_u32` / `get_i32` / `get_f32` / `get_bool` /
`iter` / `len` / `is_empty` / `into_map`。缺参数时 `get_u32` 之类返回
`Error::MissingArgument`。

### 超时变体

```rust
service.action_with_timeout(url, "GetVolume", args, Duration::from_secs(5)).await?;
service.action_ordered_with_timeout(url, "GetPositionInfo", args, Duration::from_secs(5)).await?;
```

默认超时是 `rupnp::DEFAULT_TIMEOUT`（30 秒）。

---

## 5. 读取服务描述（SCPD）

```rust
let scpd = service.scpd(device.url()).await?;

scpd.urn();                // &URN
scpd.state_variables();     // &[Rc<StateVariable>]
scpd.actions();            // &[Action]
```

`Action`：

```rust
action.name()              // &String
action.input_arguments()   // impl Iterator<Item = &Argument>
action.output_arguments()  // impl Iterator<Item = &Argument>
```

`Argument`：

```rust
arg.name()                     // &str
arg.is_input() / arg.is_output()
arg.related_state_variable()   // &Rc<StateVariable>
```

`StateVariable`：

```rust
sv.name()            // &str
sv.kind()            // &StateVariableKind
sv.default()         // Option<&str>
sv.optional()        // bool
sv.sends_events()    // bool
sv.is_multicast()    // bool
```

`StateVariableKind`：

```rust
StateVariableKind::Simple(DataType)   // ui1 / i4 / String / Uri …
StateVariableKind::Enum(Vec<String>)   // 枚举可选值
StateVariableKind::Range(StateVariableRange)
```

`DataType` 是 UPnP 定义的数据类型枚举（`ui1` `ui2` `ui4` `ui8` `i1` `i2` `i4` `int`
`r4` `r8` `Number` `Float` `Fixed14_4` `Char` `String` `Date` `DateTime` `DateTimeTz`
`Time` `TimeTz` `Boolean` `BinBase64` `BinHex` `Uri`）。

带超时的版本：`service.scpd_with_timeout(url, Duration::from_secs(5)).await?`。

---

## 6. 事件订阅

需要 `subscribe` feature（默认开启）。

```rust
let (sid, mut stream) = service.subscribe(device.url(), 300).await?;

while let Some(state_vars) = stream.try_next().await? {
    for (key, value) in state_vars {
        println!("{key} => {value}");
    }
}
```

- `timeout_secs` 是放进 `SUBSCRIBE` 请求的 `TIMEOUT` 头（`Second-300`），
  设备会在这个时间后重新发初始事件
- 返回的 `sid` 用于续订和退订

### 续订与退订

```rust
service.renew_subscription(device.url(), &sid, 300).await?;
service.unsubscribe(device.url(), &sid).await?;
```

### 设备不支持订阅

有些设备不提供 `eventSubURL`。这时服务依然能正常解析和使用，只是不能订阅：

```rust
if !service.supports_events() {
    println!("该服务不支持事件订阅");
}
```

强行订阅会得到 `Error::EventSubscriptionNotSupported`。

### 多网卡机器：指定回调地址

事件是靠设备**反向回调**本机送过来的，所以回调 IP 必须是设备能访问到的。
自动选择会优先挑与设备同网段的网卡，但在 VPN / 虚拟网卡很多的环境下仍可能选错：

```rust
use std::net::IpAddr;

let callback_ip: IpAddr = "192.168.1.10".parse().unwrap();
let (sid, mut stream) = service
    .subscribe_with_callback(device.url(), callback_ip, 300)
    .await?;
```

想看本机有哪些可用地址：

```rust
for ip in rupnp::utils::get_local_addrs()? {
    println!("{ip}");
}
```

---

## 7. XML 转义

```rust
use rupnp::xml::{escape_attr, escape_text};

escape_text("Fish & Chips <b>")   // "Fish &amp; Chips &lt;b&gt;"
escape_attr("a \"quote\"")        // "a &quot;quote&quot;"
```

`escape_text` 额外把回车转成 `&#xD;`（XML 解析器会把字面回车归一化成换行），
`escape_attr` 额外处理制表 / 换行 / 回车（属性值归一化）。

---

## 8. DIDL-Lite 元数据

DLNA 渲染器需要 DIDL-Lite 来显示媒体标题。

```rust
use rupnp::didl::{class, Didl, DidlItem, DidlResource};

let didl = Didl::new()
    .item(
        DidlItem::new()
            .id("1")
            .title("Big Buck Bunny")
            .class(class::VIDEO)                 // object.item.videoItem
            .creator("Blender Foundation")
            .album_art_uri("http://h/cover.jpg")
            .resource(
                DidlResource::new(
                    "http-get:*:video/mp4:*",    // protocolInfo
                    "http://h/movie.mp4",         // 媒体地址
                )
                .size(12_345_678)
                .duration("0:10:34.000"),
            ),
    )
    .to_xml();
```

### 常用项

| `DidlItem` | 说明 |
|---|---|
| `id` / `parent_id` / `restricted` | 通用属性，默认 `"0"` / `"-1"` / `true` |
| `title` | `dc:title` |
| `class` | `upnp:class`，用 `didl::class::*` 常量 |
| `creator` | `dc:creator` |
| `album_art_uri` | `upnp:albumArtURI` |
| `resource` | 加一个 `res` 元素 |
| `element` | 加任意元素，见下 |

| `DidlResource` | 说明 |
|---|---|
| `new(protocol_info, uri)` | 必填的两个属性 |
| `duration` / `size` / `resolution` / `bitrate` | 常用可选属性 |
| `attribute(name, value)` | 任意属性 |

`didl::class` 提供的常量：

```
VIDEO           object.item.videoItem
VIDEO_RECORDING object.item.videoItem.videoRecording
AUDIO           object.item.audioItem.musicTrack
AUDIO_RECORDING object.item.audioItem.audioRecording
IMAGE           object.item.imageItem.photo
```

### 任意命名空间的元素

```rust
use rupnp::didl::{DidlElement, Namespace};

DidlItem::new().element(DidlElement::new(
    Namespace::Upnp,
    "artist",
    "Some Artist",
))
```

`Namespace` 枚举限定了可用的前缀（`Dc` / `Upnp` / `Dlna` / `Sec`），
避免拼错命名空间。

### 输出形式

```rust
didl.to_xml()           // 可嵌入其它文档的片段（无 xml 声明）
didl.to_xml_document()  // 带 xml 声明的独立文档
```

---

## 9. AVTransport（投屏）

`rupnp::avtransport` 把 AVTransport 服务的常用动作封装成了强类型 API。

```rust
use rupnp::avtransport::Cast;

let service = device.find_service(&AV_TRANSPORT).unwrap();
let cast = Cast::new(service.clone(), device.url().clone());

cast.set_uri("http://h/movie.mp4", Some(&didl)).await?;   // SetAVTransportURI
cast.play().await?;                                        // Play
cast.pause().await?;                                       // Pause
cast.stop().await?;                                        // Stop
cast.seek(SeekUnit::RelTime, "0:01:30").await?;            // Seek
```

### 读状态

```rust
let state = cast.state().await?;                  // TransportState
let info = cast.transport_info().await?;          // TransportInfo
let position = cast.position_info().await?;       // PositionInfo
```

`TransportState`：

| 变体 | 设备上报的值 |
|---|---|
| `Idle` | `IDLE` |
| `Loading` | `LOADING` |
| `Playing` | `PLAYING` / `STREAMING` |
| `Paused` | `PAUSED_PLAYBACK` |
| `Stopped` | `STOPPED` / `STREAMING_STOPPED` |
| `Transitioning` | `TRANSITIONING` |
| `Buffering` | `STREAMING_BUFFERING` |
| `NoMedia` | `NO_MEDIA_PRESENT` |
| `Unknown` | 其它 |

`state.is_playing()` 是便捷判断。

`PositionInfo` 字段：`track` / `track_duration` / `track_meta_data` / `track_uri` /
`rel_time` / `abs_time` / `rel_count` / `abs_count`。
`position_info.duration()` 与 `position_info.position()` 会把 UPnP 的
`H+:MM:SS.F+` 格式解析成 `std::time::Duration`。

### 等待状态

```rust
use std::time::Duration;

cast.wait_for_state(TransportState::Playing, Duration::from_secs(10)).await?;
```

超时返回 `Error::StateNotReached`。

### 手动拼 payload

需要本库没封装的动作时，用这些自由函数：

```rust
use rupnp::avtransport;

avtransport::set_av_transport_uri_payload(0, "http://h/movie.mp4", Some(&didl));
avtransport::play_payload(0, 1.0);
avtransport::pause_payload(0);
avtransport::stop_payload(0);
avtransport::seek_payload(0, "REL_TIME", "0:01:30");
```

---

## 10. 错误处理

```rust
use rupnp::Error;

match service.action(device.url(), "GetVolume", args).await {
    Ok(response) => { /* ... */ }
    Err(Error::UPnPError(err)) => {
        // 设备返回了 SOAP Fault
        eprintln!("设备拒绝: {} ({})", err.err_code(), err.err_code_description());
    }
    Err(Error::HttpErrorCode(code)) => eprintln!("HTTP {code}"),
    Err(Error::Timeout) => eprintln!("设备无响应"),
    Err(err) => eprintln!("其它错误: {err}"),
}
```

`Error` 是 `#[non_exhaustive]`，匹配时需要兜底分支。

设备返回 SOAP Fault 时，`err_code()` 给出 UPnP 标准错误码，
`err_code_description()` 给出可读描述（401/402/501/600/601/602 …）。

---

## 11. Feature flags

```toml
[dependencies]
rupnp = "3"                                    # 默认含 subscribe

rupnp = { version = "3", default-features = false }              # 不含事件订阅
rupnp = { version = "3", features = ["full_device_spec"] }       # 完整设备字段
```

| feature | 默认 | 作用 |
|---|---|---|
| `subscribe` | ✅ | 事件订阅。额外依赖 `genawaiter`、`if-addrs` |
| `full_device_spec` | ❌ | 厂商、型号、UDN 等字段作为结构体字段暴露 |

不使用事件订阅时关掉 `subscribe` 可以少编译几个依赖。
注意 `subscribe` 与 `full_device_spec` 不是可加的组合，
`--no-default-features --features subscribe` 要单独测过。

---

# 媒体工具（`rupnp::mp4_tool`）

这一组是处理本地媒体文件的辅助能力，和 UPnP 协议无关，可以单独用。
场景说明见 [用例 7–11](use-cases.md#用例-7媒体处理分割--拼接--插入--生成空白片)。

## 12. 准备 ffmpeg

几乎所有媒体操作都要 ffmpeg。`ensure_ffmpeg` 负责把它准备好：

```rust
use rupnp::mp4_tool::{self, FfmpegOptions};

// 默认：本机没有就下载一个
let ffmpeg = mp4_tool::ensure_ffmpeg(&FfmpegOptions::default())?;

// 指定已有的 ffmpeg，不会下载
let ffmpeg = mp4_tool::ensure_ffmpeg(&FfmpegOptions {
    executable: Some("/usr/local/bin/ffmpeg".into()),
    ..FfmpegOptions::default()
})?;
```

`FfmpegOptions`：

| 字段 | 作用 |
|---|---|
| `executable` | 已有的 ffmpeg 路径。给了就直接用，**不下载**；不存在则报错 |
| `url` | ffmpeg 的下载地址。为空时用 `ffmpeg-sidecar` 内置的地址 |
| `directory` | 下载到哪个目录。为空时用 sidecar 目录 |

都没指定时会**创建下载目录**再下载。下载走 Rust 的 HTTP 客户端
（`ffmpeg-sidecar` 用的 `ureq`），**不依赖系统里有没有 `curl`**；失败会重试并打日志。

国内网络直连境外源很慢（实测 gyan.dev 只有 11 KB/s），可以用镜像：

```rust
let ffmpeg = mp4_tool::ensure_ffmpeg(&FfmpegOptions {
    url: Some(
        "https://gh-proxy.com/https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip".into(),
    ),
    ..FfmpegOptions::default()
})?;
```

## 13. 检查与修复 MP4 布局

渲染器通过 HTTP Range 拉流时，如果 `moov` 在 `mdat` 之后，渲染器必须把整个文件
下载完才能开始播。`Layout` 用来判断，`remux` 用来修：

```rust
use std::path::Path;
use rupnp::mp4_tool;

// 读顶层 atom
let atoms = mp4_tool::top_level_atoms(Path::new("movie.mp4"))?;

for atom in &atoms {
    println!("{} @ {} ({} 字节)", atom.kind, atom.offset, atom.size);
}

// 判断布局
match mp4_tool::layout(&atoms) {
    mp4_tool::Layout::FastStart => println!("已经优化过了"),
    mp4_tool::Layout::MoovAtEnd => println!("moov 在末尾，渲染器要下完整个文件"),
    mp4_tool::Layout::Unknown => println!("不是标准 MP4"),
}

// 修复：把 moov 搬到文件头
mp4_tool::remux(&ffmpeg, Path::new("movie.mp4"), Path::new("movie.faststart.mp4"))?;
```

`Atom` 有 `offset` / `kind` / `size` 三个字段，`Layout` 是
`FastStart` / `MoovAtEnd` / `Unknown`。

## 14. 时间格式

`parse_time` 解析 ffmpeg 的时间写法，媒体相关的函数都用它：

```rust
use std::time::Duration;
use rupnp::mp4_tool::parse_time;

assert_eq!(parse_time("90"), Some(Duration::from_secs(90)));
assert_eq!(parse_time("1:30"), Some(Duration::from_secs(90)));
assert_eq!(parse_time("00:01:30.500"), Some(Duration::from_millis(90_500)));
assert_eq!(parse_time("1h30m"), Some(Duration::from_secs(5400)));
assert_eq!(parse_time("abc"), None);
```

| 写法 | 含义 |
|---|---|
| `90`、`1.5` | 秒 |
| `1:30` | 分:秒 |
| `00:01:30` | 时:分:秒 |
| `00:01:30.500` | 带小数秒 |
| `90s`、`2m`、`1h30m` | 带单位 |

## 15. 分割 / 拼接 / 插入 / 空白片

```rust
use std::path::{Path, PathBuf};
use rupnp::mp4_tool::{self, MediaKind};

// 分割：从起点开始取一段时间
let out = mp4_tool::split(&ffmpeg, Path::new("movie.mp4"), "00:00:30", "60")?;

// 拼接：按顺序接起来
let out = mp4_tool::concat(&ffmpeg, &[PathBuf::from("a.mp4"), PathBuf::from("b.mp4")])?;

// 插入：在指定时间插入一段
let out = mp4_tool::insert(
    &ffmpeg,
    Path::new("movie.mp4"),
    Path::new("ad.mp4"),
    "00:05:00",
)?;

// 生成黑屏视频 / 静音音频
let out = mp4_tool::blank(&ffmpeg, MediaKind::Video, "5")?;
let out = mp4_tool::blank(&ffmpeg, MediaKind::Audio, "5")?;
```

`concat` 和 `insert` 要求输入是**同一种容器和编码**（它们只搬运数据，不转码）。

`MediaKind::from_path` 按扩展名判断类型（`.mp4` / `.mkv` → `Video`，
`.mp3` / `.flac` → `Audio`）。

## 16. 转码

```rust
use std::path::Path;
use rupnp::mp4_tool::{self, TranscodeOptions};

// 先探测格式
let info = mp4_tool::probe(&ffmpeg, Path::new("movie.mkv"))?;
println!("{} / {:?} / {:?}", info.format, info.video_codec, info.audio_codec);

// 转码
let outcome = mp4_tool::to_mp4(
    &ffmpeg,
    Path::new("movie.mkv"),
    Path::new("movie.converted.mp4"),
    &TranscodeOptions::default(),
)?;

println!(
    "remuxed={} 视频重编码={} 音频重编码={}",
    outcome.remuxed, outcome.reencoded_video, outcome.reencoded_audio
);
```

`to_mp4` 会先探测，**能 copy 的流就不转码**：已经是 MP4 + H.264 + AAC 时只换容器，
秒级完成。

`TranscodeOptions`：

| 字段 | 默认 | 说明 |
|---|---|---|
| `preset` | `veryfast` | x264 preset，`ultrafast` … `veryslow` |
| `crf` | 23 | 越小画质越好体积越大 |
| `audio_bitrate` | `128k` | 音频码率 |
| `pixel_format` | `yuv420p` | 兼容性最好 |
| `faststart` | `true` | moov 前置，投屏必需 |
| `max_height` | `None` | 等比缩小到该高度以内 |

预设：`TranscodeOptions::fast()`（`ultrafast` / crf 28）、
`TranscodeOptions::high_quality()`（`slow` / crf 20 / 192k）。

批量：

```rust
let results = mp4_tool::to_mp4_batch(
    &ffmpeg,
    &[PathBuf::from("a.avi"), PathBuf::from("b.flv")],
    &TranscodeOptions::default(),
);
```

`MediaInfo` 带 `format` / `video_codec` / `audio_codec` / `duration` /
`sample_rate` / `channels`。

## 17. 音频：提取 / 转换 / 拼接 / 分割

```rust
use std::path::{Path, PathBuf};
use std::time::Duration;
use rupnp::mp4_tool::{self, ConcatMp3Options, FadeCurve, Mp3Options, SplitAudioOptions};

// 从视频提取音轨
let out = mp4_tool::extract_mp3(&ffmpeg, Path::new("movie.mp4"), None, &Mp3Options::default())?;

// WAV 等转 MP3
let out = mp4_tool::to_mp3(&ffmpeg, Path::new("song.wav"), None, &Mp3Options::high())?;

// 拼接，带间隔和淡入淡出
let options = ConcatMp3Options {
    gap: Duration::from_secs(1),
    fade: Duration::from_millis(500),
    curve: FadeCurve::Sine,
    ..ConcatMp3Options::default()
};
let out = mp4_tool::concat_mp3(
    &ffmpeg,
    &[PathBuf::from("a.mp3"), PathBuf::from("b.mp3")],
    Path::new("playlist.mp3"),
    &options,
)?;

// 截取 [开始, 结束)
let out = mp4_tool::split_audio(
    &ffmpeg,
    Path::new("song.mp3"),
    "00:00:30",
    "00:01:30",
    None,
    &SplitAudioOptions::default(),
)?;
```

`Mp3Options` 预设：

| 预设 | 码率 | 采样率 | 声道 | 用途 |
|---|---|---|---|---|
| `default()` | 192k | 原样 | 原样 | 一般用途 |
| `high()` | 320k | 原样 | 原样 | 接近无损 |
| `voice()` | 64k | 44100 | 单声道 | 语音、播客 |

`ConcatMp3Options` 预设：`seamless()`（无缝）、`with_gap_and_fade(gap, fade)`。

`FadeCurve`：`Linear`（线性）、`Sine`（默认，听感自然）、`Exponential`。

`SplitAudioOptions` 只有 `reencode`（默认 `false`，用 `-c copy` 快且无损）和 `mp3`。

从视频提取时如果文件没有音轨，会在调用 ffmpeg 之前就报「里没有音轨」，
退出码非 0 且不留半成品文件。
