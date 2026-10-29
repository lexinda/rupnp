//! Cast a media file to a DLNA renderer on the local network.
//!
//! Usage: `cargo run --example cast -- <media url> [title]`
//!
//! The media url has to be reachable by the renderer, so it usually points at an http
//! server running on this machine.

use futures::prelude::*;
use rupnp::avtransport;
use rupnp::didl::{class, Didl, DidlItem, DidlResource};
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
        let payload = avtransport::set_av_transport_uri_payload(0, &media_url, Some(&didl));

        service
            .action(device.url(), "SetAVTransportURI", &payload)
            .await?;
        service
            .action(device.url(), "Play", &avtransport::play_payload(0, 1.0))
            .await?;

        // Subscribe to the transport state to follow the playback.
        let (_sid, mut stream) = service.subscribe(device.url(), 300).await?;

        while let Some(state_vars) = stream.try_next().await? {
            for (key, value) in state_vars {
                println!("  {key} => {value}");
            }
        }

        return Ok(());
    }

    println!("No renderer with an AVTransport service found.");

    Ok(())
}
