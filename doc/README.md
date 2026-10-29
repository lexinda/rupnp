# rupnp 文档

`rupnp` 是一个异步的 UPnP **控制点（Control Point）** 库，用于在局域网中发现设备、调用设备的
action、读取服务描述，以及订阅设备的状态变化事件。

> 规范：[UPnP Device Architecture v2.0](http://upnp.org/specs/arch/UPnP-arch-DeviceArchitecture-v2.0.pdf)

## 文档导航

| 文档 | 内容 |
|---|---|
| [架构](architecture.md) | 本库在 UPnP 协议栈中的位置、模块分层、数据流、URL 解析模型、错误与超时模型 |
| [使用方法](usage.md) | 全部公开 API 的用法说明：发现、调用 action、读 SCPD、事件订阅、XML/DIDL 构造 |
| [用例](use-cases.md) | 端到端场景：局域网投屏、投文档、状态查询、事件同步、自定义 action |

## 用例索引

| # | 场景 | 难度 | 关键点 |
|---|---|---|---|
| 1 | [局域网投屏（DLNA Play-To）](use-cases.md#用例-1局域网投屏dlna-play-to) | 中 | 控制面本库搞定，媒体面要自己写 **支持 Range 的** HTTP 服务器 |
| 2 | [设备状态查询与音量控制](use-cases.md#用例-2设备状态查询与音量控制) | 易 | 先读 SCPD 确认参数顺序再拼 payload |
| 3 | [事件订阅与状态同步](use-cases.md#用例-3事件订阅与状态同步) | 中 | 记得续订；多网卡要显式指定回调 IP |
| 4 | [调用自定义 action](use-cases.md#用例-4调用自定义-action) | 中 | 参数值要自己转义 |
| 5 | [批量发现与设备筛选](use-cases.md#用例-5批量发现与设备筛选) | 易 | `services_iter()` / `devices_iter()` 是递归的 |
| 6 | [投文档（PPT / Word）](use-cases.md#用例-6投文档ppt--word) | 中 | 标准 DLNA 不支持直接投，要先转成图片或视频 |

## 三十秒上手

```rust
use futures::prelude::*;
use rupnp::ssdp::{SearchTarget, URN};
use std::time::Duration;

const RENDERING_CONTROL: URN = URN::service("schemas-upnp-org", "RenderingControl", 1);

#[tokio::main]
async fn main() -> Result<(), rupnp::Error> {
    let devices = rupnp::discover(&SearchTarget::RootDevice, Duration::from_secs(3), None).await?;
    let mut devices = std::pin::pin!(devices);

    while let Some(device) = devices.try_next().await? {
        let service = device
            .find_service(&RENDERING_CONTROL)
            .expect("searched for RenderingControl, got something else");

        let args = "<InstanceID>0</InstanceID><Channel>Master</Channel>";
        let response = service.action(device.url(), "GetVolume", args).await?;

        println!("'{}' is at volume {}", device.friendly_name(), response["CurrentVolume"]);
    }

    Ok(())
}
```

## 能力边界

本库只实现**控制面**，不实现媒体面与被控端。

| 能做 | 不能做 |
|---|---|
| SSDP 发现设备（`discover`） | 提供 HTTP 文件服务（投屏媒体服务器要自己写） |
| 解析设备描述与服务描述 | 实现 DMR / MediaRenderer 服务端（被电视投屏） |
| 调用任意 SOAP action | Miracast / Wi-Fi Display 屏幕镜像 |
| 订阅 GENA 事件 | Chromecast（Cast 协议）、AirPlay |
| 构造 DIDL-Lite 元数据 | IPv6 组播事件（multicast） |

典型的 **DLNA 投屏**（把本地视频推给电视）是可以做的，但需要你自己另外提供一个
支持 HTTP Range 的媒体服务器 —— 详见 [用例 1](use-cases.md#用例-1局域网投屏dlna-play-to)。

## 快速索引

| 我想…… | 看 |
|---|---|
| 搞清楚模块是怎么分层的 | [架构](architecture.md) |
| 查某个 API 怎么用 | [使用方法](usage.md) |
| 抄一段能跑的代码 | [用例](use-cases.md) |
| 找 examples | [`../examples/`](../examples/) |

## 例子

| 文件 | 说明 |
|---|---|
| [`discover.rs`](../examples/discover.rs) | 列出局域网里的所有设备 |
| [`discover_extra_fields.rs`](../examples/discover_extra_fields.rs) | 发现时顺带读取厂商等扩展属性 |
| [`search_and_action.rs`](../examples/search_and_action.rs) | 按服务类型发现并调用 action |
| [`rendering_control.rs`](../examples/rendering_control.rs) | 直接用设备 URL 读取音量 |
| [`dump_scpd.rs`](../examples/dump_scpd.rs) | 导出所有设备的 SCPD，便于离线分析 |
| [`subscribe.rs`](../examples/subscribe.rs) | 订阅事件，打印状态变化 |
| [`cast.rs`](../examples/cast.rs) | 把一个媒体地址投到 DLNA 渲染器 |

> examples 是**手工验证工具**，不会被 `cargo test` 执行，因为它们需要局域网里真实存在
> UPnP 设备。可自动化跑的测试是 `cargo test` 覆盖的单元测试与文档测试。
