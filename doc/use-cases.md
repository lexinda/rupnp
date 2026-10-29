# 鐢ㄤ緥

姣忎釜鐢ㄤ緥閮芥槸鍙繍琛岀殑瀹屾暣浠ｇ爜銆傞粯璁ゅ凡 `use futures::prelude::*;`銆?
---

## 鐢ㄤ緥 1锛氬眬鍩熺綉鎶曞睆锛圖LNA Play-To锛?
鎶婃湰鏈虹殑瑙嗛鎺ㄧ粰灞€鍩熺綉閲岀殑 DLNA 娓叉煋鍣紙鐢佃 / 鐩掑瓙 / 闊崇锛夈€?
### 1.1 鏁翠綋娴佺▼

```
鈹屸攢鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹?  鈶燬SDP 鎼滅储    鈹屸攢鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹?鈹? 浣犵殑绋嬪簭 鈹?鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈻?鈹? 娓叉煋鍣?(DMR)     鈹?鈹?(鎺у埗鐐?  鈹?鈼勨攢鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€ 鈹? AVTransport:1    鈹?鈹斺攢鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹?  鈶℃媺鍙栨弿杩?     鈹斺攢鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹?     鈹?                             鈻?     鈹?鈶etAVTransportURI + Play    鈹?鈶TTP GET (Range)
     鈹傗攢鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈻衡攤
     鈹?                             鈹?鈶ユ媺娴佹挱鏀?     鈹?鈶ET 浣犵殑 MP4 鏈嶅姟鍣?鈼勨攢鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹?```

**鎺у埗闈紙鈶犫憽鈶級鐢辨湰搴撳畬鎴愶紱濯掍綋闈紙鈶も懃锛夌敱浣犺嚜宸辩殑 HTTP 鏈嶅姟鍣ㄥ畬鎴愩€?*

### 1.2 鎺у埗绔唬鐮?
```rust
use rupnp::avtransport::Cast;
use rupnp::didl::{class, Didl, DidlItem, DidlResource};
use rupnp::ssdp::{SearchTarget, URN};
use std::time::Duration;

const AV_TRANSPORT: URN = URN::service("schemas-upnp-org", "AVTransport", 1);

#[tokio::main]
async fn main() -> Result<(), rupnp::Error> {
    // 濯掍綋鍦板潃蹇呴』鑳借娓叉煋鍣ㄨ闂埌
    let media_url = "http://192.168.1.10:8080/movie.mp4";

    // 鈶?鎼滅储甯?AVTransport 鐨勮澶?    //    SearchTarget::URN(...) 鏄嚱鏁拌皟鐢紝浼氫骇鐢熶复鏃跺€硷紝蹇呴』鍏堢粦瀹氬埌鍙橀噺
    let search_target = SearchTarget::URN(AV_TRANSPORT);
    let devices = rupnp::discover(&search_target, Duration::from_secs(3), None).await?;
    let mut devices = std::pin::pin!(devices);

    while let Some(device) = devices.try_next().await? {
        let Some(service) = device.find_service(&AV_TRANSPORT) else {
            continue;
        };

        println!("鎶曞睆鍒?{}", device.friendly_name());

        let cast = Cast::new(service.clone(), device.url().clone());

        // 鈶?鏋勯€?DIDL-Lite 鍏冩暟鎹紙鍐冲畾鐢佃涓婃樉绀虹殑鏍囬锛?        let didl = Didl::new()
            .item(
                DidlItem::new()
                    .title("鎴戠殑鐢靛奖")
                    .class(class::VIDEO)
                    .resource(DidlResource::new("http-get:*:video/mp4:*", media_url)),
            )
            .to_xml();

        // 鈶?涓嬪彂鎺ㄦ祦骞舵挱鏀?        cast.set_uri(media_url, Some(&didl)).await?;
        cast.play().await?;

        // 鈶?璺熻釜鎾斁杩涘害
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

### 1.3 濯掍綋鏈嶅姟鍣細蹇呴』鏀寔 HTTP Range

**杩欐槸鎶曞睆鏈€甯歌鐨勫け璐ョ偣銆?* DLNA 娓叉煋鍣ㄧ殑鏍囧噯琛屼负鏄細

```
HEAD /movie.mp4                    鈫?鎺㈡祴 Content-Length / Accept-Ranges
GET  /movie.mp4  Range: bytes=0-   鈫?206 Partial Content锛屽紑濮嬫媺娴?GET  /movie.mp4  Range: bytes=N-   鈫?206锛宻eek 鏃剁敤
```

涓嶆敮鎸?Range 鐨勬湇鍔″櫒鍦ㄦ覆鏌撳櫒涓婄殑琛ㄧ幇閫氬父鏄細鐘舵€佸崱鍦?`TRANSITIONING` 涓嶅姩銆?鎾嚑绉掑悗鍋溿€佹垨鑰?`CurrentTransportStatus` 鍙樻垚 `ERROR_OCCURRED`銆?
涓€涓鐢ㄧ殑 Range 鏈嶅姟鍣紙鏀惧埌浣犺嚜宸辩殑 crate 閲岋級锛?
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

/// 鍦?`port` 涓婃彁渚?`data`锛屾敮鎸?Range 璇锋眰銆?async fn serve(data: Bytes, port: u16) -> std::io::Result<()> {
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

    // 娓叉煋鍣ㄤ竴鑸厛鍙?HEAD 鎺㈡祴鏂囦欢
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

/// 瑙ｆ瀽 `bytes=start-end` 鑼冨洿澶淬€?fn parse_range(value: &str) -> Option<(u64, u64)> {
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

闇€瑕佸湪浣犺嚜宸辩殑 `Cargo.toml` 閲屽姞锛?
```toml
[dependencies]
hyper = { version = "1", features = ["server", "http1"] }
hyper-util = { version = "0.1", features = ["tokio", "server", "http1"] }
http-body-util = "0.1"
bytes = "1"
tokio = { version = "1", features = ["rt-multi-thread", "net", "fs"] }
```

### 1.4 濯掍綋鏈嶅姟鍣ㄧ殑鍏跺畠瑕佹眰

| 瑕佹眰 | 璇存槑 |
|---|---|
| 鐩戝惉 `0.0.0.0` | 缁?`127.0.0.1` 璁惧璁块棶涓嶅埌 |
| 鍦板潃鐢ㄦ湰鏈哄眬鍩熺綉 IP | `http://192.168.1.10:8080/movie.mp4`锛屼笉鑳界敤 `localhost` |
| `Content-Type` | `video/mp4`锛岃鍜?DIDL 閲岀殑 `protocolInfo` 瀵瑰緱涓?|
| `Accept-Ranges: bytes` | 鏄庣‘鍛婅瘔璁惧鏀寔 Range |
| `HEAD` 鏀寔 | 寰堝璁惧鍏堟帰娴?|
| 鏂囦欢鍙В鐮?| H.264 / AAC 杩欑被甯歌 MP4 娌￠棶棰?|

### 1.5 鎺掓煡

| 鐜拌薄 | 鍘熷洜 |
|---|---|
| `SetAVTransportURI` 杩斿洖 400 / 501 | 鍙傛暟椤哄簭涓嶅锛屾垨 DIDL-Lite 涓嶅悎娉?鈫?鐢?`service.scpd(url)` 鏍稿鍙傛暟椤哄簭 |
| 鐘舵€佸崱鍦?`TRANSITIONING` | 濯掍綋鏈嶅姟鍣ㄤ笉鍙揪锛屾垨涓嶆敮鎸?Range 鈫?鐪嬫湇鍔″櫒鏃ュ織鏈夋病鏈夋敹鍒拌姹?|
| `CurrentTransportStatus` = `ERROR_OCCURRED` | 璁惧鎷夋祦澶辫触 鈫?澶氬崐鏄牸寮忎笉鏀寔鎴栨湇鍔″櫒杩斿洖浜嗛潪 2xx |
| `find_service` 杩斿洖 `None` | 璁惧娌℃湁 AVTransport 鏈嶅姟锛屾垨鎻忚堪鎹熷潖锛堜細琚烦杩囧苟鎵?warn 鏃ュ織锛?|
| 鐢佃涓婁笉鏄剧ず鏍囬 | DIDL-Lite 娌′紶锛屾垨 `dc:title` 鍛藉悕绌洪棿娌″０鏄?|
| 浜嬩欢鏀朵笉鍒?| 鍥炶皟 IP 閫夐敊锛堝缃戝崱锛夆啋 鐢?`subscribe_with_callback` 鏄惧紡鎸囧畾 |

---

## 鐢ㄤ緥 2锛氳澶囩姸鎬佹煡璇笌闊抽噺鎺у埗

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

        // 璇诲綋鍓嶉煶閲?        let args = "<InstanceID>0</InstanceID><Channel>Master</Channel>";
        let response = service.action(device.url(), "GetVolume", args).await?;
        let volume: u16 = response["CurrentVolume"].parse().unwrap_or(0);
        println!("褰撳墠闊抽噺 {volume}");

        // 璋冮煶閲?        let args = "<InstanceID>0</InstanceID><Channel>Master</Channel><DesiredVolume>30</DesiredVolume>";
        service.action(device.url(), "SetVolume", args).await?;
    }

    Ok(())
}
```

### 璇?SCPD 纭鍙傛暟鍚嶄笌椤哄簭

涓嶅悓鍘傚晢鐨勫弬鏁板悕鍙兘涓嶅悓锛屽厛璇绘湇鍔℃弿杩版渶淇濋櫓锛?
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

## 鐢ㄤ緥 3锛氫簨浠惰闃呬笌鐘舵€佸悓姝?
```rust
use futures::prelude::*;
use rupnp::ssdp::URN;
use std::time::Duration;

const AV_TRANSPORT: URN = URN::service("schemas-upnp-org", "AVTransport", 1);

#[tokio::main]
async fn main() -> Result<(), rupnp::Error> {
    let device = /* 鍙戠幇鍒扮殑璁惧 */;
    let service = device.find_service(&AV_TRANSPORT).unwrap();

    if !service.supports_events() {
        println!("璇ユ湇鍔′笉鏀寔浜嬩欢璁㈤槄");
        return Ok(());
    }

    // 璁㈤槄 300 绉?    let (sid, mut stream) = service.subscribe(device.url(), 300).await?;

    // 鏀跺埌浜嬩欢鏃剁画璁紝閬垮厤璁㈤槄杩囨湡
    while let Some(state_vars) = stream.try_next().await? {
        for (key, value) in state_vars {
            println!("{key} => {value}");
        }

        service.renew_subscription(device.url(), &sid, 300).await?;
    }

    Ok(())
}
```

### 浜嬩欢閲屾嬁鍒扮殑鏄粈涔?
浜嬩欢鏄?`<e:propertyset>`锛屾湰搴撴妸瀹冭В鏋愭垚 `HashMap<String, String>`銆?閿槸鐘舵€佸彉閲忓悕锛屽€兼槸瀛楃涓层€侫VTransport 鐨?`LastChange` 鍙橀噺鍊兼湰韬張鏄竴娈?XML锛屽舰濡傦細

```xml
<Event xmlns="urn:schemas-upnp-org:metadata-1-0:AVT/">
  <InstanceID val="0">
    <TransportState val="PLAYING"/>
  </InstanceID>
</Event>
```

闇€瑕佹洿缁嗙殑缁撴瀯灏辫嚜宸辩敤 `rupnp::xml` / `roxmltree` 浜屾瑙ｆ瀽銆?
### 澶氱綉鍗＄幆澧?
璁惧闈犲弽鍚戝洖璋冮€佷簨浠讹紝鍥炶皟 IP 蹇呴』璁惧鑳借闂埌銆傝嚜鍔ㄩ€夋嫨浼氫紭鍏堟寫鍚岀綉娈电殑缃戝崱锛?浣?VPN / 铏氭嫙缃戝崱澶氱殑鏈哄櫒涓婁粛鍙兘閫夐敊锛?
```rust
use std::net::IpAddr;

let callback_ip: IpAddr = "192.168.1.10".parse().unwrap();
let (sid, mut stream) = service
    .subscribe_with_callback(device.url(), callback_ip, 300)
    .await?;
```

---

## 鐢ㄤ緥 4锛氳皟鐢ㄨ嚜瀹氫箟 action

鏈変簺璁惧鎻愪緵鍘傚晢鑷畾涔夌殑鏈嶅姟锛屽弬鏁板湪鏍囧噯涔嬪銆傚仛娉曚竴鏍凤紝鍙槸鍙傛暟瑕佽嚜宸辨嫾锛?
```rust
let service = device
        .find_service(&URN::service("schemas-xxx-com", "Custom", 1))
        .unwrap();

// 鍏堣 SCPD 鎼炴竻妤氬弬鏁?let scpd = service.scpd(device.url()).await?;
for action in scpd.actions() {
    println!("{}: {} 涓叆鍙?, action.name(), action.input_arguments().count());
}

// 鎷煎弬鏁帮紙璁板緱杞箟锛?let payload = format!(
    "<Param>{}</Param>",
    rupnp::xml::escape_text("value & more")
);
let response = service.action_ordered(device.url(), "CustomAction", &payload).await?;

for (name, value) in response.iter() {
    println!("{name} = {value}");
}
```

---

## 鐢ㄤ緥 5锛氭壒閲忓彂鐜颁笌璁惧绛涢€?
```rust
use futures::prelude::*;
use rupnp::ssdp::{SearchTarget, URN};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), rupnp::Error> {
    let devices =
        rupnp::discover(&SearchTarget::RootDevice, Duration::from_secs(5), None).await?;
    let mut devices = std::pin::pin!(devices);

    // 璁惧鎻忚堪鏄苟鍙戞媺鍙栫殑锛屼骇鍑虹殑椤哄簭涓?SSDP 鍝嶅簲椤哄簭涓€鑷?    while let Some(device) = devices.try_next().await? {
        // 鍙甯?AVTransport 鐨?        if device.find_service(&URN::service("schemas-upnp-org", "AVTransport", 1)).is_none() {
            continue;
        }

        println!(
            "{} [{}] {}",
            device.friendly_name(),
            device.device_type().typ(),
            device.url(),
        );

        // 鍒楀嚭瀹冩彁渚涚殑鎵€鏈夋湇鍔?        for service in device.services_iter() {
            println!("    {}", service.service_type().typ());
        }
    }

    Ok(())
}
```

### 璇诲彇鍘傚晢绛夋墿灞曞瓧娈?
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

### 瀵煎嚭 SCPD 鍋氱绾垮垎鏋?
浠撳簱閲岀殑 `examples/dump_scpd.rs` 浼氭妸灞€鍩熺綉閲屾墍鏈夎澶囩殑 SCPD 瀵煎嚭鍒?`descriptions/` 鐩綍锛屾柟渚垮鐓т笉鍚屽巶鍟嗙殑瀹炵幇宸紓銆?