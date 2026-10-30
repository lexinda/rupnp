//! 把同一个媒体投到局域网里的多个 DLNA 渲染器。
//!
//! 用法: `cargo run --example cast_group -- <媒体地址>`
//!
//! 媒体地址必须能被渲染器访问到。

use futures::prelude::*;
use rupnp::avtransport::TransportState;
use rupnp::cast_group::CastGroup;
use rupnp::ssdp::SearchTarget;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), rupnp::Error> {
    let media_url = std::env::args()
        .nth(1)
        .expect("用法: cast_group <媒体地址>");

    // 1. 发现所有设备
    let devices = rupnp::discover(&SearchTarget::RootDevice, Duration::from_secs(5), None).await?;
    let mut devices = std::pin::pin!(devices);

    let mut all = Vec::new();
    while let Some(device) = devices.try_next().await? {
        all.push(device);
    }

    // 2. 挑出带 AVTransport 服务的设备
    let group = CastGroup::from_devices(&all);

    println!("发现 {} 个设备，其中 {} 个可以投屏", all.len(), group.len());
    for member in group.members() {
        println!("  - {}", member.name);
    }

    if group.is_empty() {
        println!("没有可以投屏的设备");
        return Ok(());
    }

    // 3. 把同一个地址下发给所有设备
    let outcome = group.set_uri_and_play(&media_url, None).await;

    let failures: Vec<String> = outcome
        .failures()
        .map(|(name, err)| format!("{name}: {err}"))
        .collect();

    for failure in &failures {
        eprintln!("  失败 {failure}");
    }

    println!(
        "  下发完成：{} 个成功，{} 个失败",
        outcome.succeeded(),
        outcome.failed()
    );

    // 4. 等所有设备都播起来
    if let Err(err) = group
        .wait_for_state(TransportState::Playing, Duration::from_secs(10))
        .await
    {
        eprintln!("等待播放状态失败: {err}");
    }

    // 5. 打印每台设备的进度
    loop {
        let states = group.states().await;

        for (member, state) in group.members().iter().zip(states) {
            println!("  {}: {:?}", member.name, state);
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
