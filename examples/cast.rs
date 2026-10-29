//! Cast a media file to a DLNA renderer on the local network.
//!
//! Usage: `cargo run --example cast -- <media url> [title]`
//!
//! The media url has to be reachable by the renderer, so it usually points at an http
//! server running on this machine.

use futures::prelude::*;
use rupnp::avtransport::Cast;
use rupnp::didl::{Didl, DidlItem, DidlResource, class};
use rupnp::ssdp::{SearchTarget, URN};
use std::time::Duration;

const AV_TRANSPORT: URN = URN::service("schemas-upnp-org", "AVTransport", 1);

#[tokio::main]
async fn main() -> Result<(), rupnp::Error> {
    let mut args = std::env::args().skip(1);
    let media_url = args.next().expect("usage: cast <media url> [title]");
    let title = args.next().unwrap_or_else(|| "Media".to_string());

    let devices = rupnp::discover(&SearchTarget::RootDevice, Duration::from_secs(3), None).await?;
    let mut devices = std::pin::pin!(devices);

    while let Some(device) = devices.try_next().await? {
        let Some(service) = device.find_service(&AV_TRANSPORT) else {
            continue;
        };

        println!("Casting to {} ({})", device.friendly_name(), device.url());

        let cast = Cast::new(service.clone(), device.url().clone());

        let didl = Didl::new()
            .item(
                DidlItem::new()
                    .title(title.clone())
                    .class(class::VIDEO)
                    .resource(DidlResource::new(
                        "http-get:*:video/mp4:*",
                        media_url.clone(),
                    )),
            )
            .to_xml();

        // The metadata is escaped by the helper, so it can be embedded into the payload.
        cast.set_uri(&media_url, Some(&didl)).await?;
        cast.play().await?;

        // Follow the playback until the media is done.
        loop {
            let info = cast.position_info().await?;

            println!(
                "  track {} of {} at {} / {}",
                info.track, info.abs_count, info.rel_time, info.track_duration
            );

            if !cast.state().await?.is_playing() {
                break;
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        return Ok(());
    }

    println!("No renderer with an AVTransport service found.");

    Ok(())
}
