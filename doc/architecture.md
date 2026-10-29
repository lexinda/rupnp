# 架构

## 1. 在 UPnP 协议栈中的位置

```
┌──────────────────────────────────────────────────────────────┐
│  你的应用（投屏 / 媒体控制 / 设备管理）                        │
└───────────────────────────┬──────────────────────────────────┘
                            │ rupnp
┌───────────────────────────▼──────────────────────────────────┐
│  控制点 Control Point（本库）                                  │
│  ├─ SSDP 发现（ssdp-client）                                  │
│  ├─ 设备描述解析（roxmltree）                                 │
│  ├─ SOAP action 调用（hyper / hyper-util）                     │
│  ├─ GENA 事件订阅（tokio TcpListener）                        │
│  └─ DIDL-Lite / XML 构造                                      │
└───────────────────────────┬──────────────────────────────────┘
                            │ HTTP + SOAP
        ┌───────────────────┴───────────────────┐
        │                                       │
┌───────▼──────────────┐            ┌───────────▼──────────────┐
│ 媒体渲染器 DMR        │            │ 媒体服务器 DMS            │
│ (AVTransport 等)      │            │ (ContentDirectory 等)      │
└───────────────────────┘            └──────────────────────────┘
```

本库是**控制点**：它主动去发现别人、请求别人、订阅别人的事件。
反过来「被投屏」（电视把画面投到手机）需要实现一个 DMR 服务端，本库不提供。

协议栈里的对应关系：

| UPnP 概念 | 本库对应物 |
|---|---|
| SSDP M-SEARCH / NOTIFY | `rupnp::discover`（底层是 `ssdp_client`） |
| Device Description | `rupnp::Device` / `rupnp::DeviceSpec` |
| Service Description (SCPD) | `rupnp::scpd::SCPD` |
| Control URL (SOAP) | `rupnp::Service::action` |
| Eventing (GENA) | `rupnp::Service::subscribe` |
| DIDL-Lite 元数据 | `rupnp::didl` |

---

## 2. 模块分层

```
                    ┌─────────────────────────────┐
   应用层            │  avtransport（投屏语义封装）   │
                    └──────────────┬──────────────┘
                                   │
                    ┌──────────────▼──────────────┐
   领域层            │  didl   │  xml            │
                    │  DIDL-Lite 构造 │ XML 转义   │
                    └──────────────┬──────────────┘
                                   │
   协议层            ┌──────────────▼──────────────┐
                    │  Service   │  Device        │
                    │  SOAP action │ 描述解析      │
                    │  GENA 事件   │               │
                    └──────────────┬──────────────┘
                                   │
   传输层            ┌──────────────▼──────────────┐
                    │  client（共享 http client）  │
                    │  utils（URL 解析 / 超时）    │
                    └─────────────────────────────┘
```

| 模块 | 职责 | 对外可见 |
|---|---|---|
| `discovery` | SSDP 搜索，拿到设备描述 URL，并发拉取描述 | `discover`, `discover_with_properties` |
| `device` | 设备描述解析、设备树、`URLBase` 处理 | `Device`, `DeviceSpec` |
| `service` | 服务描述、`controlURL` 解析、SOAP action、GENA 订阅 | `Service` |
| `scpd` | 服务控制协议描述（action / argument / state variable） | `scpd::SCPD` 等 |
| `action` | action 响应（保序 + 类型化） | `ActionResponse` |
| `avtransport` | AVTransport 语义封装：投屏控制器、状态枚举 | `Cast`, `TransportState` 等 |
| `didl` | DIDL-Lite 元数据构造 | `Didl` 等 |
| `xml` | XML 文本 / 属性转义 | `escape_text`, `escape_attr` |
| `error` | 统一错误类型 | `Error`, `UPnPError` |
| `client` | 共享 http client（内部） | — |
| `utils` | URL 解析、base url、超时包装等工具 | `base_url`, `resolve_url` 等 |

---

## 3. 关键数据流

### 3.1 发现

```
discover(search_target, timeout, ttl)
   │
   ├─ ssdp_client::search()          发出 M-SEARCH，收集 NOTIFY 响应
   │                                 每个响应带一个 LOCATION（设备描述 URL）
   │
   └─ .buffered(8)                   并发拉取设备描述（保持 SSDP 响应顺序）
        │
        └─ Device::from_url_and_properties()
             │
             ├─ GET <LOCATION>
             ├─ 解析 <root><device> → DeviceSpec
             │    ├─ 读取 <URLBase>（可选）→ base url
             │    ├─ 递归解析 <deviceList> 子设备
             │    └─ 解析 <serviceList> → Service
             │         └─ 校验 SCPDURL / controlURL / eventSubURL 是否是合法 url 引用
             └─ 返回 Device
```

`buffered` 而不是 `buffer_unordered` 是刻意的：前者保持 SSDP 响应顺序，
设备产出的顺序与改动前一致，不会打乱依赖顺序的调用方。

### 3.2 调用 action

```
Service::action(url, "GetVolume", payload)
   │
   ├─ 拼 SOAP Envelope：<u:{action} xmlns:u="{serviceType}">{payload}</u:{action}>
   ├─ 头：Content-Type: text/xml; charset="utf-8"
   │      SOAPAction: "urn:…:service:RenderingControl:1#GetVolume"
   │
   ├─ control_url = resolve(设备描述 base url, controlURL)
   ├─ POST control_url ──► 设备
   │                      ◄── HTTP 200 + SOAP 响应
   │
   ├─ 若状态码 != 200              → Error::HttpErrorCode
   ├─ 若响应是 <s:Fault>           → Error::UPnPError（含 errorCode）
   └─ 否则收集子元素 → ActionResponse（保序）
```

**payload 是原样拼进 SOAP Body 的，库不替你转义。** 参数里带 `&`（比如媒体 URL 带
查询参数）必须自己用 `rupnp::xml::escape_text` 处理，否则设备会收到非法 XML。

### 3.3 事件订阅

```
Service::subscribe(url, timeout_secs)
   │
   ├─ 选本机回调地址：pick_local_addr(peer_ip(url))
   │    优先选与设备同网段的网卡，避免多网卡时选错 VPN / 虚拟网卡
   ├─ TcpListener::bind(callback_ip, 0)      监听随机端口
   ├─ SUBSCRIBE eventSubURL
   │    头：CALLBACK: <http://本机ip:端口>   NT: upnp:event   TIMEOUT: Second-N
   │    ◄── 200 OK + SID 头
   │
   └─ 返回 (sid, stream)
        设备事件到达 ──► 接受连接 ──► 读 HTTP 头 ──► 读 <e:propertyset>
                          ──► 回 "HTTP/1.1 200 OK\r\n\r\n" ──► 解析成 HashMap
                          ──► yield 给调用方
```

---

## 4. URL 解析模型

这是最容易出错的地方，值得单独说明。

设备描述里的 `SCPDURL` / `controlURL` / `eventSubURL` 是 **url 引用**，可能是三种形态：

| 形态 | 例子 | 解析结果（设备描述在 `http://h:1400/xml/device_description.xml`） |
|---|---|---|
| 绝对 url | `http://other:1400/control` | 原样使用 |
| 根相对 | `/upnp/control/avtr` | `http://h:1400/upnp/control/avtr` |
| 相对 | `MediaRenderer/control` | `http://h:1400/xml/MediaRenderer/control` |

**base url 的来源**（优先级从高到低）：

1. 根设备的 `<URLBase>` 元素 —— 设备显式指定基准
2. 设备描述文档 URL 去掉最后一段路径 —— `…/xml/device_description.xml` → `…/xml/`

实现见 `utils::base_url` 与 `utils::resolve_url`（遵循 RFC 3986，含点分段移除）。
`http::Uri` 自身**不支持相对引用**（`MediaRenderer/control` 会直接解析失败），
所以这套解析是手写的。

`Service` 保存的是**原始 url 引用**，在调用时才相对传入的设备 url 解析。
这样 `action(device.url(), …)` 里的 url 参数仍然有意义，公开 API 也不需要改。

---

## 5. HTTP 客户端与连接复用

库内部用 `OnceLock` 共享**一个** http client（`client` 模块），
所有对设备的请求都走它，因此：

- 连接池生效，对同一设备的连续请求会复用 TCP 连接
- 投屏这类连续下发命令的场景（`SetAVTransportURI` → `Play` → `Stop`）不会每次重新握手

客户端跑在 tokio runtime 上（`hyper_util::rt::TokioExecutor`），
所以 **tokio 是事实上的必需依赖**，即使不启用 `subscribe` feature。

---

## 6. 超时模型

| 层次 | 机制 | 默认 |
|---|---|---|
| SSDP 搜索 | `ssdp_client::search` 的 `timeout` 参数 | 调用方指定 |
| 设备描述 / SCPD / action / 订阅 | `tokio::time::timeout` 包住整个请求 | `rupnp::DEFAULT_TIMEOUT` = 30s |
| 投屏等待状态 | `Cast::wait_for_state` 轮询 | 调用方指定 |

`DEFAULT_TIMEOUT` 是常量而非全局可变状态：需要别的值就用 `*_with_timeout` 变体
（`action_with_timeout`、`from_url_with_timeout`、`scpd_with_timeout` …）。

超时的错误是 `Error::Timeout`。

---

## 7. 错误模型

所有公开 API 返回 `Result<T, rupnp::Error>`。`Error` 是 `#[non_exhaustive]`，
所以将来加变体不会破坏下游。

| 变体 | 含义 |
|---|---|
| `UPnPError(UPnPError)` | 设备返回了 SOAP Fault，带 `errorCode`（401/402/501/600…） |
| `SSDPError` | SSDP 搜索失败 |
| `NetworkError` / `NetworkClientError` | 连接层失败 |
| `HttpErrorCode(StatusCode)` | 设备回了非 200 |
| `XmlError` / `XmlMissingElement` | XML 解析失败 / 缺少必需元素 |
| `InvalidResponse` | 设备返回了无法解析的值 |
| `InvalidUrl` / `InvalidUtf8` / `ParseError` | 各类格式错误 |
| `Timeout` | 请求超时 |
| `MissingArgument(String)` | action 响应里没有请求的参数 |
| `StateNotReached(String)` | 渲染器没在超时内到达目标状态 |
| `EventSubscriptionNotSupported` | 服务没有 `eventSubURL`，无法订阅 |
| `NoLocalInterfaceOpen` | 找不到可用的本机网卡 |

**设备描述的容错**：某个 service 描述损坏时会被跳过并打 `warn` 日志，
而不是让整台设备解析失败 —— 一个坏 service 不应该让整台设备不可用。

---

## 8. 运行时模型

- 需要 **tokio runtime**（客户端用 `TokioExecutor`，超时用 `tokio::time`）
- 所有 I/O 都是 async，API 全部返回 future
- `Service`、`Device`、`ActionResponse` 都是 `Clone + Send + Sync`
- 事件订阅用 `genawaiter` 的 `Gen` 生成一个 `Stream`

---

## 9. Feature flags

| feature | 默认 | 作用 |
|---|---|---|
| `subscribe` | ✅ | 事件订阅能力。额外依赖 `genawaiter`、`if-addrs`，以及 tokio 的 `net` / `io-util` |
| `full_device_spec` | ❌ | 把厂商、型号、UDN 等字段作为结构体字段暴露出来（否则只有友好名和设备类型） |

`default = ["subscribe"]`。

由于 feature 不是可加的，`--no-default-features` 与 `--features subscribe`
必须分别测。CI 里已经用矩阵覆盖了全部组合。
