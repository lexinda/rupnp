# 浣跨敤鏂规硶

鏈枃瑕嗙洊鍏ㄩ儴鍏紑 API銆傜ず渚嬮粯璁?`use futures::prelude::*;`锛堜负浜?`try_next`锛変笌
`use std::time::Duration;`銆?
---

## 1. 鍙戠幇璁惧

```rust
use futures::prelude::*;
use rupnp::ssdp::{SearchTarget, URN};
use std::time::Duration;

// 鎼滅储鎵€鏈夋牴璁惧
let devices = rupnp::discover(&SearchTarget::RootDevice, Duration::from_secs(3), None).await?;
let mut devices = std::pin::pin!(devices);

while let Some(device) = devices.try_next().await? {
    println!("{} - {}", device.friendly_name(), device.url());
}
```

`discover` 杩斿洖鐨勬槸娴侊紝蹇呴』 `pin` 浣忔墠鑳?`try_next`銆?
### 鎼滅储鐗瑰畾绫诲瀷鐨勬湇鍔?
姣旀悳鏍硅澶囨洿绮惧噯锛屼篃鏇村揩锛?
```rust
const AV_TRANSPORT: URN = URN::service("schemas-upnp-org", "AVTransport", 1);

// 娉ㄦ剰锛歋earchTarget::URN(...) 鏄嚱鏁拌皟鐢紝浼氫骇鐢熶复鏃跺€笺€?// discover 杩斿洖鐨勬祦鍊熺敤浜嗗畠锛屾墍浠ュ繀椤诲厛缁戝畾鍒板彉閲忥紝鍚﹀垯缂栬瘧涓嶈繃銆?let search_target = SearchTarget::URN(AV_TRANSPORT);
let devices = rupnp::discover(&search_target, Duration::from_secs(3), None).await?;
```

> `SearchTarget::RootDevice` 鏄父閲忥紝鍙互鐩存帴鍐?`&SearchTarget::RootDevice`銆?
### 鍙傛暟

```rust
pub async fn discover(
    search_target: &SearchTarget,
    timeout: Duration,      // SSDP 鎼滅储鏃堕暱
    ttl: Option<u32>,       // 鏀捐繘 M-SEARCH 鐨?TTL 瀛楁
) -> Result<impl Stream<Item = Result<Device>>>
```

`ttl` 鏄?SSDP 鎶ユ枃鐨?TTL锛堢锛夛紝鏈変簺璁惧浼氭嵁姝ゅ喅瀹氬洖澶嶅嚑涓寘銆?
### 椤哄甫璇诲彇鎵╁睍灞炴€?
`full_device_spec` feature 鍙毚闇插浐瀹氱殑涓€鎵瑰瓧娈点€備换鎰忓睘鎬х敤
`discover_with_properties`锛?
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

> 娉ㄦ剰 `Device` 閫氳繃 `Deref` 鎸囧悜 `DeviceSpec`锛屾墍浠?`device.get_extra_property(...)`
> 鐩存帴鍙敤銆?
---

## 2. 宸茬煡 URL 鐩存帴璇昏澶?
SSDP 鏈夋椂涓嶅彲鐢紙缁勬挱琚銆佽澶囦笉鏀寔 M-SEARCH锛夛紝杩欐椂鐩存帴鎷挎弿杩?URL锛?
```rust
use rupnp::{http::Uri, Device};

let url = Uri::from_static("http://192.168.1.5:1400/xml/device_description.xml");
let device = Device::from_url(url.clone()).await?;

println!("{}", device.friendly_name());
```

甯﹁秴鏃跺拰鎵╁睍灞炴€х殑鍙樹綋锛?
```rust
let device = Device::from_url_with_timeout(url.clone(), Duration::from_secs(5)).await?;
let device = Device::from_url_and_properties(url.clone(), &["manufacturer"]).await?;
let device = Device::from_url_and_properties_with_timeout(
    url,
    &["manufacturer"],
    Duration::from_secs(5),
).await?;
```

> `Uri` 涓嶆槸 `Copy`锛屽娆¤皟鐢ㄩ渶瑕?`url.clone()`銆?
---

## 3. 璁惧鎻忚堪涓庤澶囨爲

`Device` = 鎻忚堪 URL + 璁惧鎻忚堪鍐呭銆傚唴瀹瑰湪 `DeviceSpec` 閲岋紝`Device` 閫氳繃
`Deref` 鏆撮湶瀹冦€?
```rust
device.device_type()        // &URN锛岃澶囩被鍨?device.friendly_name()      // &str
device.get_extra_property("manufacturer")   // Option<&str>
```

### 閬嶅巻瀛愯澶囦笌鏈嶅姟

```rust
// 鐩存帴瀛愯澶?/ 鐩存帴鏈嶅姟
device.devices()      // &Vec<DeviceSpec>
device.services()     // &Vec<Service>

// 閫掑綊閬嶅巻鎵€鏈夊眰绾?device.devices_iter()  // impl Iterator<Item = &DeviceSpec>
device.services_iter() // impl Iterator<Item = &Service>

// 鎸夌被鍨嬫煡鎵?device.find_service(&URN::service("schemas-upnp-org", "AVTransport", 1))
device.find_device(&URN::device("schemas-upnp-org", "MediaRenderer", 1))
```

`find_service` / `find_device` 閮芥槸**閫掑綊**鐨勶紝浼氭悳鍒板瓙璁惧閲岀殑銆?
### 瀹屾暣璁惧瀛楁锛坄full_device_spec` feature锛?
鍚敤鍚庨澶栧彲鐢細

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

## 4. 璋冪敤 action

鎵€鏈?action 鐨勮皟鐢ㄦ柟寮忛兘涓€鏍凤細鏈嶅姟 + 鐩爣 URL + action 鍚?+ 鍙傛暟鐗囨銆?
```rust
let service = device.find_service(&AV_TRANSPORT).unwrap();

let args = "<InstanceID>0</InstanceID><CurrentURI>http://h/movie.mp4</CurrentURI>";
let response = service.action(device.url(), "SetAVTransportURI", args).await?;

println!("{:?}", response);
```

杩斿洖鐨勬槸 `HashMap<String, String>`锛岄敭鏄?action 鐨勮緭鍑哄弬鏁板悕銆?
### 鍙傛暟椤哄簭

**鍙傛暟鐗囨蹇呴』鎸夋湇鍔℃弿杩帮紙SCPD锛夐噷 `argumentList` 鐨勯『搴忔嫾鎺?*锛岃繖鏄?UPnP 鐨勭‖鎬ц姹傘€?鍙互鍏堢敤 `scpd()` 璇诲嚭鍙傛暟椤哄簭锛?
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

### 杞箟鍙傛暟鍊?
**payload 鏄師鏍锋嫾杩?SOAP Body 鐨勶紝搴撲笉鏇夸綘杞箟銆?* 鍚?`&` `<` `>` 鐨勫€煎繀椤昏嚜宸卞鐞嗭細

```rust
use rupnp::xml::escape_text;

let uri = "http://h/movie.mp4?a=1&b=2";
let args = format!(
    "<InstanceID>0</InstanceID><CurrentURI>{}</CurrentURI>",
    escape_text(uri),
);
```

`escape_text` 鐢ㄤ簬鍏冪礌鍐呭锛宍escape_attr` 鐢ㄤ簬灞炴€у€硷紙棰濆澶勭悊寮曞彿涓庣┖鐧藉瓧绗︼級銆?
### 绫诲瀷鍖栥€佷繚搴忕殑鍝嶅簲

`HashMap` 涓㈠け鍙傛暟椤哄簭锛屼篃娌℃湁绫诲瀷銆傜敤 `action_ordered`锛?
```rust
let response = service.action_ordered(device.url(), "GetPositionInfo", args).await?;

let track: u32 = response.get_u32("Track")?;
let rel_time = response.get("RelTime").unwrap_or_default();
let playing = response.get_bool("Enabled")?;

// 淇濆簭閬嶅巻
for (name, value) in response.iter() {
    println!("{name} = {value}");
}
```

`ActionResponse` 鎻愪緵 `get` / `get_u32` / `get_i32` / `get_f32` / `get_bool` /
`iter` / `len` / `is_empty` / `into_map`銆傜己鍙傛暟鏃?`get_u32` 涔嬬被杩斿洖
`Error::MissingArgument`銆?
### 瓒呮椂鍙樹綋

```rust
service.action_with_timeout(url, "GetVolume", args, Duration::from_secs(5)).await?;
service.action_ordered_with_timeout(url, "GetPositionInfo", args, Duration::from_secs(5)).await?;
```

榛樿瓒呮椂鏄?`rupnp::DEFAULT_TIMEOUT`锛?0 绉掞級銆?
---

## 5. 璇诲彇鏈嶅姟鎻忚堪锛圫CPD锛?
```rust
let scpd = service.scpd(device.url()).await?;

scpd.urn();                // &URN
scpd.state_variables();     // &[Rc<StateVariable>]
scpd.actions();            // &[Action]
```

`Action`锛?
```rust
action.name()              // &String
action.input_arguments()   // impl Iterator<Item = &Argument>
action.output_arguments()  // impl Iterator<Item = &Argument>
```

`Argument`锛?
```rust
arg.name()                     // &str
arg.is_input() / arg.is_output()
arg.related_state_variable()   // &Rc<StateVariable>
```

`StateVariable`锛?
```rust
sv.name()            // &str
sv.kind()            // &StateVariableKind
sv.default()         // Option<&str>
sv.optional()        // bool
sv.sends_events()    // bool
sv.is_multicast()    // bool
```

`StateVariableKind`锛?
```rust
StateVariableKind::Simple(DataType)   // ui1 / i4 / String / Uri 鈥?StateVariableKind::Enum(Vec<String>)   // 鏋氫妇鍙€夊€?StateVariableKind::Range(StateVariableRange)
```

`DataType` 鏄?UPnP 瀹氫箟鐨勬暟鎹被鍨嬫灇涓撅紙`ui1` `ui2` `ui4` `ui8` `i1` `i2` `i4` `int`
`r4` `r8` `Number` `Float` `Fixed14_4` `Char` `String` `Date` `DateTime` `DateTimeTz`
`Time` `TimeTz` `Boolean` `BinBase64` `BinHex` `Uri`锛夈€?
甯﹁秴鏃剁殑鐗堟湰锛歚service.scpd_with_timeout(url, Duration::from_secs(5)).await?`銆?
---

## 6. 浜嬩欢璁㈤槄

闇€瑕?`subscribe` feature锛堥粯璁ゅ紑鍚級銆?
```rust
let (sid, mut stream) = service.subscribe(device.url(), 300).await?;

while let Some(state_vars) = stream.try_next().await? {
    for (key, value) in state_vars {
        println!("{key} => {value}");
    }
}
```

- `timeout_secs` 鏄斁杩?`SUBSCRIBE` 璇锋眰鐨?`TIMEOUT` 澶达紙`Second-300`锛夛紝
  璁惧浼氬湪杩欎釜鏃堕棿鍚庨噸鏂板彂鍒濆浜嬩欢
- 杩斿洖鐨?`sid` 鐢ㄤ簬缁鍜岄€€璁?
### 缁涓庨€€璁?
```rust
service.renew_subscription(device.url(), &sid, 300).await?;
service.unsubscribe(device.url(), &sid).await?;
```

### 璁惧涓嶆敮鎸佽闃?
鏈変簺璁惧涓嶆彁渚?`eventSubURL`銆傝繖鏃舵湇鍔′緷鐒惰兘姝ｅ父瑙ｆ瀽鍜屼娇鐢紝鍙槸涓嶈兘璁㈤槄锛?
```rust
if !service.supports_events() {
    println!("璇ユ湇鍔′笉鏀寔浜嬩欢璁㈤槄");
}
```

寮鸿璁㈤槄浼氬緱鍒?`Error::EventSubscriptionNotSupported`銆?
### 澶氱綉鍗℃満鍣細鎸囧畾鍥炶皟鍦板潃

浜嬩欢鏄潬璁惧**鍙嶅悜鍥炶皟**鏈満閫佽繃鏉ョ殑锛屾墍浠ュ洖璋?IP 蹇呴』鏄澶囪兘璁块棶鍒扮殑銆?鑷姩閫夋嫨浼氫紭鍏堟寫涓庤澶囧悓缃戞鐨勭綉鍗★紝浣嗗湪 VPN / 铏氭嫙缃戝崱寰堝鐨勭幆澧冧笅浠嶅彲鑳介€夐敊锛?
```rust
use std::net::IpAddr;

let callback_ip: IpAddr = "192.168.1.10".parse().unwrap();
let (sid, mut stream) = service
    .subscribe_with_callback(device.url(), callback_ip, 300)
    .await?;
```

鎯崇湅鏈満鏈夊摢浜涘彲鐢ㄥ湴鍧€锛?
```rust
for ip in rupnp::utils::get_local_addrs()? {
    println!("{ip}");
}
```

---

## 7. XML 杞箟

```rust
use rupnp::xml::{escape_attr, escape_text};

escape_text("Fish & Chips <b>")   // "Fish &amp; Chips &lt;b&gt;"
escape_attr("a \"quote\"")        // "a &quot;quote&quot;"
```

`escape_text` 棰濆鎶婂洖杞﹁浆鎴?`&#xD;`锛圶ML 瑙ｆ瀽鍣ㄤ細鎶婂瓧闈㈠洖杞﹀綊涓€鍖栨垚鎹㈣锛夛紝
`escape_attr` 棰濆澶勭悊鍒惰〃 / 鎹㈣ / 鍥炶溅锛堝睘鎬у€煎綊涓€鍖栵級銆?
---

## 8. DIDL-Lite 鍏冩暟鎹?
DLNA 娓叉煋鍣ㄩ渶瑕?DIDL-Lite 鏉ユ樉绀哄獟浣撴爣棰樸€?
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
                    "http://h/movie.mp4",         // 濯掍綋鍦板潃
                )
                .size(12_345_678)
                .duration("0:10:34.000"),
            ),
    )
    .to_xml();
```

### 甯哥敤椤?
| `DidlItem` | 璇存槑 |
|---|---|
| `id` / `parent_id` / `restricted` | 閫氱敤灞炴€э紝榛樿 `"0"` / `"-1"` / `true` |
| `title` | `dc:title` |
| `class` | `upnp:class`锛岀敤 `didl::class::*` 甯搁噺 |
| `creator` | `dc:creator` |
| `album_art_uri` | `upnp:albumArtURI` |
| `resource` | 鍔犱竴涓?`res` 鍏冪礌 |
| `element` | 鍔犱换鎰忓厓绱狅紝瑙佷笅 |

| `DidlResource` | 璇存槑 |
|---|---|
| `new(protocol_info, uri)` | 蹇呭～鐨勪袱涓睘鎬?|
| `duration` / `size` / `resolution` / `bitrate` | 甯哥敤鍙€夊睘鎬?|
| `attribute(name, value)` | 浠绘剰灞炴€?|

`didl::class` 鎻愪緵鐨勫父閲忥細

```
VIDEO           object.item.videoItem
VIDEO_RECORDING object.item.videoItem.videoRecording
AUDIO           object.item.audioItem.musicTrack
AUDIO_RECORDING object.item.audioItem.audioRecording
IMAGE           object.item.imageItem.photo
```

### 浠绘剰鍛藉悕绌洪棿鐨勫厓绱?
```rust
use rupnp::didl::{DidlElement, Namespace};

DidlItem::new().element(DidlElement::new(
    Namespace::Upnp,
    "artist",
    "Some Artist",
))
```

`Namespace` 鏋氫妇闄愬畾浜嗗彲鐢ㄧ殑鍓嶇紑锛坄Dc` / `Upnp` / `Dlna` / `Sec`锛夛紝
閬垮厤鎷奸敊鍛藉悕绌洪棿銆?
### 杈撳嚭褰㈠紡

```rust
didl.to_xml()           // 鍙祵鍏ュ叾瀹冩枃妗ｇ殑鐗囨锛堟棤 xml 澹版槑锛?didl.to_xml_document()  // 甯?xml 澹版槑鐨勭嫭绔嬫枃妗?```

---

## 9. AVTransport锛堟姇灞忥級

`rupnp::avtransport` 鎶?AVTransport 鏈嶅姟鐨勫父鐢ㄥ姩浣滃皝瑁呮垚浜嗗己绫诲瀷 API銆?
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

### 璇荤姸鎬?
```rust
let state = cast.state().await?;                  // TransportState
let info = cast.transport_info().await?;          // TransportInfo
let position = cast.position_info().await?;       // PositionInfo
```

`TransportState`锛?
| 鍙樹綋 | 璁惧涓婃姤鐨勫€?|
|---|---|
| `Idle` | `IDLE` |
| `Loading` | `LOADING` |
| `Playing` | `PLAYING` / `STREAMING` |
| `Paused` | `PAUSED_PLAYBACK` |
| `Stopped` | `STOPPED` / `STREAMING_STOPPED` |
| `Transitioning` | `TRANSITIONING` |
| `Buffering` | `STREAMING_BUFFERING` |
| `NoMedia` | `NO_MEDIA_PRESENT` |
| `Unknown` | 鍏跺畠 |

`state.is_playing()` 鏄究鎹峰垽鏂€?
`PositionInfo` 瀛楁锛歚track` / `track_duration` / `track_meta_data` / `track_uri` /
`rel_time` / `abs_time` / `rel_count` / `abs_count`銆?`position_info.duration()` 涓?`position_info.position()` 浼氭妸 UPnP 鐨?`H+:MM:SS.F+` 鏍煎紡瑙ｆ瀽鎴?`std::time::Duration`銆?
### 绛夊緟鐘舵€?
```rust
use std::time::Duration;

cast.wait_for_state(TransportState::Playing, Duration::from_secs(10)).await?;
```

瓒呮椂杩斿洖 `Error::StateNotReached`銆?
### 鎵嬪姩鎷?payload

闇€瑕佹湰搴撴病灏佽鐨勫姩浣滄椂锛岀敤杩欎簺鑷敱鍑芥暟锛?
```rust
use rupnp::avtransport;

avtransport::set_av_transport_uri_payload(0, "http://h/movie.mp4", Some(&didl));
avtransport::play_payload(0, 1.0);
avtransport::pause_payload(0);
avtransport::stop_payload(0);
avtransport::seek_payload(0, "REL_TIME", "0:01:30");
```

---

## 10. 閿欒澶勭悊

```rust
use rupnp::Error;

match service.action(device.url(), "GetVolume", args).await {
    Ok(response) => { /* ... */ }
    Err(Error::UPnPError(err)) => {
        // 璁惧杩斿洖浜?SOAP Fault
        eprintln!("璁惧鎷掔粷: {} ({})", err.err_code(), err.err_code_description());
    }
    Err(Error::HttpErrorCode(code)) => eprintln!("HTTP {code}"),
    Err(Error::Timeout) => eprintln!("璁惧鏃犲搷搴?),
    Err(err) => eprintln!("鍏跺畠閿欒: {err}"),
}
```

`Error` 鏄?`#[non_exhaustive]`锛屽尮閰嶆椂闇€瑕佸厹搴曞垎鏀€?
璁惧杩斿洖 SOAP Fault 鏃讹紝`err_code()` 缁欏嚭 UPnP 鏍囧噯閿欒鐮侊紝
`err_code_description()` 缁欏嚭鍙鎻忚堪锛?01/402/501/600/601/602 鈥︼級銆?
---

## 11. Feature flags

```toml
[dependencies]
rupnp = "3"                                    # 榛樿鍚?subscribe

rupnp = { version = "3", default-features = false }              # 涓嶅惈浜嬩欢璁㈤槄
rupnp = { version = "3", features = ["full_device_spec"] }       # 瀹屾暣璁惧瀛楁
```

| feature | 榛樿 | 浣滅敤 |
|---|---|---|
| `subscribe` | 鉁?| 浜嬩欢璁㈤槄銆傞澶栦緷璧?`genawaiter`銆乣if-addrs` |
| `full_device_spec` | 鉂?| 鍘傚晢銆佸瀷鍙枫€乁DN 绛夊瓧娈典綔涓虹粨鏋勪綋瀛楁鏆撮湶 |

涓嶄娇鐢ㄤ簨浠惰闃呮椂鍏虫帀 `subscribe` 鍙互灏戠紪璇戝嚑涓緷璧栥€?娉ㄦ剰 `subscribe` 涓?`full_device_spec` 涓嶆槸鍙姞鐨勭粍鍚堬紝
`--no-default-features --features subscribe` 瑕佸崟鐙祴杩囥€?