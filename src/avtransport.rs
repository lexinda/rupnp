//! Helpers for the UPnP AVTransport service.
//!
//! AVTransport is the service a media renderer exposes to play media. The functions in
//! this module build the argument payloads of its actions, which can be passed to
//! [`Service::action`](crate::Service::action).
//!
//! # Example usage:
//! ```rust,no_run
//! # use futures::prelude::*;
//! # use rupnp::avtransport;
//! # use rupnp::didl::{class, Didl, DidlItem, DidlResource};
//! # use rupnp::ssdp::URN;
//! # async fn cast_example() -> Result<(), rupnp::Error> {
//! # let device: rupnp::Device = unimplemented!();
//! let service = device
//!     .find_service(&URN::service("schemas-upnp-org", "AVTransport", 1))
//!     .expect("service exists");
//!
//! let didl = Didl::new()
//!     .item(
//!         DidlItem::new()
//!             .title("Big Buck Bunny")
//!             .class(class::VIDEO)
//!             .resource(DidlResource::new(
//!                 "http-get:*:video/mp4:*",
//!                 "http://192.168.1.10:8080/bbb.mp4",
//!             )),
//!     )
//!     .to_xml();
//!
//! let payload = avtransport::set_av_transport_uri_payload(
//!     0,
//!     "http://192.168.1.10:8080/bbb.mp4",
//!     Some(&didl),
//! );
//!
//! service
//!     .action(device.url(), "SetAVTransportURI", &payload)
//!     .await?;
//! service
//!     .action(device.url(), "Play", &avtransport::play_payload(0, 1.0))
//!     .await?;
//! # Ok(())
//! # }
//! ```

use crate::xml::escape_text;

/// The service type of the AVTransport service.
pub const AV_TRANSPORT_URN: &str = "urn:schemas-upnp-org:service:AVTransport:1";

/// Builds the argument payload of the `SetAVTransportURI` action.
///
/// `current_uri_metadata` is the DIDL-Lite metadata of the media. It is escaped so that it
/// can be embedded into the SOAP body.
pub fn set_av_transport_uri_payload(
    instance_id: u32,
    current_uri: &str,
    current_uri_metadata: Option<&str>,
) -> String {
    let mut payload = format!(
        "<InstanceID>{instance_id}</InstanceID><CurrentURI>{current_uri}</CurrentURI>",
        current_uri = escape_text(current_uri),
    );

    if let Some(metadata) = current_uri_metadata {
        payload.push_str(&format!(
            "<CurrentURIMetaData>{metadata}</CurrentURIMetaData>",
            metadata = escape_text(metadata),
        ));
    }

    payload
}

/// Builds the argument payload of the `Play` action.
pub fn play_payload(instance_id: u32, speed: f32) -> String {
    format!("<InstanceID>{instance_id}</InstanceID><Speed>{speed}</Speed>")
}

/// Builds the argument payload of the `Pause` action.
pub fn pause_payload(instance_id: u32) -> String {
    format!("<InstanceID>{instance_id}</InstanceID>")
}

/// Builds the argument payload of the `Stop` action.
pub fn stop_payload(instance_id: u32) -> String {
    format!("<InstanceID>{instance_id}</InstanceID>")
}

/// Builds the argument payload of the `Seek` action.
///
/// `unit` is one of `TRACK_NR`, `REL_TIME` or `ABS_TIME`.
pub fn seek_payload(instance_id: u32, unit: &str, target: &str) -> String {
    format!(
        "<InstanceID>{instance_id}</InstanceID><Unit>{unit}</Unit><Target>{target}</Target>",
        unit = escape_text(unit),
        target = escape_text(target),
    )
}

/// Builds the argument payload of the `SetAVTransportURI` action with an empty instance id
/// and the given uri, without any metadata.
pub fn set_uri_payload(current_uri: &str) -> String {
    set_av_transport_uri_payload(0, current_uri, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::didl::{class, Didl, DidlItem, DidlResource};
    use roxmltree::Document;

    fn parse(payload: &str) -> roxmltree::Document<'static> {
        let xml = format!("<root>{payload}</root>");
        // Leaks the xml string into the document, but the test only uses it right away.
        Document::parse(Box::leak(xml.into_boxed_str())).unwrap()
    }

    #[test]
    fn didl_metadata_survives_the_roundtrip() {
        let didl = Didl::new()
            .item(
                DidlItem::new()
                    .id("1")
                    .title("Fish & Chips")
                    .class(class::VIDEO)
                    .resource(
                        DidlResource::new(
                            "http-get:*:video/mp4:*",
                            "http://192.168.1.10:8080/a.mp4?b=1&c=2",
                        )
                        .size(42),
                    ),
            )
            .to_xml();

        let payload =
            set_av_transport_uri_payload(0, "http://192.168.1.10:8080/a.mp4?b=1&c=2", Some(&didl));

        // The payload has to be valid xml.
        let doc = parse(&payload);

        let embedded = doc
            .descendants()
            .find(|n| n.tag_name().name() == "CurrentURIMetaData")
            .unwrap()
            .text()
            .unwrap();

        // The embedded metadata has to be valid didl-lite.
        let didl_doc = Document::parse(embedded).unwrap();
        assert_eq!(didl_doc.root_element().tag_name().name(), "DIDL-Lite");

        let title = didl_doc
            .descendants()
            .find(|n| n.tag_name().name() == "title")
            .unwrap();
        assert_eq!(title.text(), Some("Fish & Chips"));

        let res = didl_doc
            .descendants()
            .find(|n| n.tag_name().name() == "res")
            .unwrap();
        assert_eq!(res.text(), Some("http://192.168.1.10:8080/a.mp4?b=1&c=2"));
        assert_eq!(res.attribute("size"), Some("42"));
    }

    #[test]
    fn set_uri_payload_contains_the_arguments() {
        let doc = parse(&set_av_transport_uri_payload(
            0,
            "http://192.168.1.10:8080/bbb.mp4",
            None,
        ));

        let instance_id = doc
            .descendants()
            .find(|n| n.tag_name().name() == "InstanceID")
            .unwrap();
        assert_eq!(instance_id.text(), Some("0"));

        let current_uri = doc
            .descendants()
            .find(|n| n.tag_name().name() == "CurrentURI")
            .unwrap();
        assert_eq!(current_uri.text(), Some("http://192.168.1.10:8080/bbb.mp4"));

        assert!(doc
            .descendants()
            .all(|n| n.tag_name().name() != "CurrentURIMetaData"));
    }

    #[test]
    fn set_uri_payload_escapes_the_uri() {
        let doc = parse(&set_av_transport_uri_payload(
            0,
            "http://192.168.1.10:8080/a.mp4?b=1&c=2",
            None,
        ));

        let current_uri = doc
            .descendants()
            .find(|n| n.tag_name().name() == "CurrentURI")
            .unwrap();
        assert_eq!(
            current_uri.text(),
            Some("http://192.168.1.10:8080/a.mp4?b=1&c=2")
        );
    }

    #[test]
    fn set_uri_payload_embeds_the_metadata() {
        let metadata = concat!(
            r#"<DIDL-Lite xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/""#,
            r#" xmlns:dc="http://purl.org/dc/elements/1.1/">"#,
            r#"<item id="1"><dc:title>a &amp; b</dc:title></item></DIDL-Lite>"#,
        );

        let doc = parse(&set_av_transport_uri_payload(
            0,
            "http://192.168.1.10:8080/bbb.mp4",
            Some(metadata),
        ));

        let embedded = doc
            .descendants()
            .find(|n| n.tag_name().name() == "CurrentURIMetaData")
            .unwrap();
        assert_eq!(embedded.text(), Some(metadata));

        // The embedded metadata has to be valid xml on its own.
        let didl = Document::parse(embedded.text().unwrap()).unwrap();
        assert_eq!(didl.root_element().tag_name().name(), "DIDL-Lite");
    }

    #[test]
    fn transport_payloads_contain_the_instance_id() {
        for payload in [play_payload(1, 1.0), pause_payload(1), stop_payload(1)] {
            let doc = parse(&payload);
            let instance_id = doc
                .descendants()
                .find(|n| n.tag_name().name() == "InstanceID")
                .unwrap();
            assert_eq!(instance_id.text(), Some("1"));
        }
    }

    #[test]
    fn play_payload_contains_the_speed() {
        let doc = parse(&play_payload(0, 2.0));

        let speed = doc
            .descendants()
            .find(|n| n.tag_name().name() == "Speed")
            .unwrap();
        assert_eq!(speed.text(), Some("2"));
    }

    #[test]
    fn seek_payload_contains_the_unit_and_the_target() {
        let doc = parse(&seek_payload(0, "REL_TIME", "0:01:30"));

        let unit = doc
            .descendants()
            .find(|n| n.tag_name().name() == "Unit")
            .unwrap();
        assert_eq!(unit.text(), Some("REL_TIME"));

        let target = doc
            .descendants()
            .find(|n| n.tag_name().name() == "Target")
            .unwrap();
        assert_eq!(target.text(), Some("0:01:30"));
    }
}
