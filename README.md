![GitHub last commit](https://img.shields.io/github/last-commit/jakobhellermann/rupnp.svg)
[![Crates.io](https://img.shields.io/crates/v/rupnp.svg)](https://crates.io/crates/rupnp)

# rupnp
An asynchronous library for finding UPnP control points, performing actions on them
and reading their service descriptions.
UPnP stand for `Universal Plug and Play` and is widely used for routers, WiFi-enabled speakers
and media servers.

Spec:
[http://rupnp.org/specs/arch/UPnP-arch-DeviceArchitecture-v2.0.pdf](http://upnp.org/specs/arch/UPnP-arch-DeviceArchitecture-v2.0.pdf)

# Documentation

| 鏂囨。 | 鍐呭 |
|---|---|
| [doc/README.md](doc/README.md) | 鏂囨。绱㈠紩涓庡揩閫熶笂鎵?|
| [doc/architecture.md](doc/architecture.md) | 鏋舵瀯锛氭ā鍧楀垎灞傘€佹暟鎹祦銆乁RL 瑙ｆ瀽妯″瀷銆侀敊璇笌瓒呮椂妯″瀷 |
| [doc/usage.md](doc/usage.md) | 浣跨敤鏂规硶锛氬叏閮ㄥ叕寮€ API 璇存槑 |
| [doc/use-cases.md](doc/use-cases.md) | 鐢ㄤ緥锛氬眬鍩熺綉鎶曞睆銆佺姸鎬佹煡璇€佷簨浠跺悓姝ャ€佽嚜瀹氫箟 action |

# Example usage:
The following code searches for devices that have a `RenderingControl` service 
and print their names along with their current volume.
```rust
use futures::prelude::*;
use std::time::Duration;
use rupnp::ssdp::{SearchTarget, URN};

const RENDERING_CONTROL: URN = URN::service("schemas-upnp-org", "RenderingControl", 1);

#[tokio::main]
async fn main() -> Result<(), rupnp::Error> {
    let search_target = SearchTarget::URN(RENDERING_CONTROL);
    let devices = rupnp::discover(&search_target, Duration::from_secs(3), None).await?;
    let mut devices = std::pin::pin!(devices);

    while let Some(device) = devices.try_next().await? {
        let service = device
            .find_service(&RENDERING_CONTROL)
            .expect("searched for RenderingControl, got something else");

        let args = "<InstanceID>0</InstanceID><Channel>Master</Channel>";
        let response = service.action(device.url(), "GetVolume", args).await?;

        let volume = response.get("CurrentVolume").unwrap();

        println!("'{}' is at volume {}", device.friendly_name(), volume);
    }

    Ok(())
}
```

License
-------

Licensed under either of

 * Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

Contribution
------------

Please use [rustfmt](https://github.com/rust-lang/rustfmt) before any pull requests.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
