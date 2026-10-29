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
use crate::{Error, Result, Service};
use http::Uri;
use std::time::Duration;

/// The service type of the AVTransport service.
pub const AV_TRANSPORT_URN: &str = "urn:schemas-upnp-org:service:AVTransport:1";

/// The interval the transport state is polled in.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// The transport state of a renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    /// The renderer is ready to play a media.
    Idle,
    /// The renderer is loading the media.
    Loading,
    /// The renderer is playing the media.
    Playing,
    /// The playback is paused.
    Paused,
    /// The playback is stopped.
    Stopped,
    /// The renderer is switching to the next media.
    Transitioning,
    /// The renderer is buffering the media.
    Buffering,
    /// There is no media to play.
    NoMedia,
    /// The renderer reported a state this library does not know.
    Unknown,
}

impl TransportState {
    /// Parses the `CurrentTransportState` of a renderer.
    pub fn from_name(state: &str) -> Self {
        match state {
            "IDLE" => TransportState::Idle,
            "LOADING" => TransportState::Loading,
            "PLAYING" | "STREAMING" => TransportState::Playing,
            "PAUSED_PLAYBACK" => TransportState::Paused,
            "STOPPED" | "STREAMING_STOPPED" => TransportState::Stopped,
            "TRANSITIONING" => TransportState::Transitioning,
            "STREAMING_BUFFERING" => TransportState::Buffering,
            "NO_MEDIA_PRESENT" => TransportState::NoMedia,
            _ => TransportState::Unknown,
        }
    }

    /// Returns the name of the state as the renderer reports it.
    pub fn as_str(self) -> &'static str {
        match self {
            TransportState::Idle => "IDLE",
            TransportState::Loading => "LOADING",
            TransportState::Playing => "PLAYING",
            TransportState::Paused => "PAUSED_PLAYBACK",
            TransportState::Stopped => "STOPPED",
            TransportState::Transitioning => "TRANSITIONING",
            TransportState::Buffering => "STREAMING_BUFFERING",
            TransportState::NoMedia => "NO_MEDIA_PRESENT",
            TransportState::Unknown => "UNDEFINED",
        }
    }

    /// Returns whether the renderer is currently playing.
    pub fn is_playing(self) -> bool {
        matches!(self, TransportState::Playing)
    }
}

impl std::str::FromStr for TransportState {
    type Err = std::convert::Infallible;

    fn from_str(state: &str) -> Result<Self, Self::Err> {
        Ok(TransportState::from_name(state))
    }
}

/// The unit a seek is relative to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekUnit {
    /// A track number.
    Track,
    /// A time relative to the start of the current track.
    RelTime,
    /// A time relative to the start of the media.
    AbsTime,
}

impl SeekUnit {
    /// Returns the name of the unit as the AVTransport service defines it.
    pub fn as_str(self) -> &'static str {
        match self {
            SeekUnit::Track => "TRACK_NR",
            SeekUnit::RelTime => "REL_TIME",
            SeekUnit::AbsTime => "ABS_TIME",
        }
    }
}

/// The information the `GetTransportInfo` action returns.
#[derive(Debug, Clone)]
pub struct TransportInfo {
    /// The current state of the renderer.
    pub current_transport_state: TransportState,
    /// The status of the current transport, for example `OK`.
    pub current_transport_status: String,
    /// The playback speed.
    pub current_speed: String,
}

/// The information the `GetPositionInfo` action returns.
///
/// Arguments the renderer does not report are empty or zero.
#[derive(Debug, Clone)]
pub struct PositionInfo {
    /// The number of the current track.
    pub track: u32,
    /// The duration of the current track.
    pub track_duration: String,
    /// The DIDL-Lite metadata of the current track.
    pub track_meta_data: String,
    /// The uri of the current track.
    pub track_uri: String,
    /// The time relative to the start of the current track.
    pub rel_time: String,
    /// The time relative to the start of the media.
    pub abs_time: String,
    /// The number of tracks before the current one.
    pub rel_count: i32,
    /// The number of tracks in total.
    pub abs_count: i32,
}

impl PositionInfo {
    /// Returns the duration of the current track.
    pub fn duration(&self) -> Option<Duration> {
        parse_duration(&self.track_duration)
    }

    /// Returns the position within the current track.
    pub fn position(&self) -> Option<Duration> {
        parse_duration(&self.rel_time)
    }
}

/// Parses a duration in the UPnP format `H+:MM:SS.F+`.
///
/// Returns `None` if the value cannot be parsed, for example because the renderer does
/// not implement the action and answers with `NOT_IMPLEMENTED`.
pub fn parse_duration(value: &str) -> Option<Duration> {
    let mut parts = value.trim().split(':');

    // The format is `H+:MM:SS.F+`, but the hours are optional.
    let seconds = parts.next_back()?.trim();
    let minutes = parts.next_back().map(str::trim);
    let hours = parts.next_back().map(str::trim);

    if parts.next().is_some() {
        return None;
    }

    let seconds: f64 = seconds.parse().ok()?;

    if !seconds.is_finite() || !(0.0..86_400.0).contains(&seconds) {
        return None;
    }

    let (hours, minutes) = match (hours, minutes) {
        (Some(hours), Some(minutes)) => (hours.parse::<u64>().ok()?, minutes.parse::<u64>().ok()?),
        (None, Some(minutes)) => (0, minutes.parse::<u64>().ok()?),
        (None, None) => (0, 0),
        (Some(_), None) => return None,
    };

    let total = Duration::from_secs(hours * 3600 + minutes * 60) + Duration::from_secs_f64(seconds);

    Some(total)
}

/// A controller for a media renderer.
///
/// It wraps the AVTransport service of a device and keeps track of the instance it
/// controls.
///
/// # Example usage:
/// ```rust,no_run
/// # use futures::prelude::*;
/// # use rupnp::avtransport::Cast;
/// # use rupnp::didl::{class, Didl, DidlItem, DidlResource};
/// # use rupnp::ssdp::URN;
/// # async fn cast_example() -> Result<(), rupnp::Error> {
/// # let device: rupnp::Device = unimplemented!();
/// let service = device
///     .find_service(&URN::service("schemas-upnp-org", "AVTransport", 1))
///     .expect("service exists");
///
/// let cast = Cast::new(service.clone(), device.url().clone());
///
/// let didl = Didl::new()
///     .item(
///         DidlItem::new()
///             .title("Big Buck Bunny")
///             .class(class::VIDEO)
///             .resource(DidlResource::new(
///                 "http-get:*:video/mp4:*",
///                 "http://192.168.1.10:8080/bbb.mp4",
///             )),
///     )
///     .to_xml();
///
/// cast.set_uri("http://192.168.1.10:8080/bbb.mp4", Some(&didl)).await?;
/// cast.play().await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Cast {
    service: Service,
    url: Uri,
    instance_id: u32,
}

impl Cast {
    /// Creates a controller for the given AVTransport service.
    pub fn new(service: Service, url: Uri) -> Self {
        Self {
            service,
            url,
            instance_id: 0,
        }
    }

    /// Creates a controller for a specific instance of the AVTransport service.
    pub fn with_instance_id(mut self, instance_id: u32) -> Self {
        self.instance_id = instance_id;
        self
    }

    /// Returns the instance this controller controls.
    pub fn instance_id(&self) -> u32 {
        self.instance_id
    }

    /// Returns the AVTransport service of the renderer.
    pub fn service(&self) -> &Service {
        &self.service
    }

    /// Sets the media the renderer should play.
    ///
    /// `metadata` is the DIDL-Lite metadata of the media.
    pub async fn set_uri(&self, uri: &str, metadata: Option<&str>) -> Result<()> {
        let payload = set_av_transport_uri_payload(self.instance_id, uri, metadata);

        self.service
            .action(&self.url, "SetAVTransportURI", &payload)
            .await?;

        Ok(())
    }

    /// Starts or resumes the playback.
    pub async fn play(&self) -> Result<()> {
        self.service
            .action(&self.url, "Play", &play_payload(self.instance_id, 1.0))
            .await?;

        Ok(())
    }

    /// Pauses the playback.
    pub async fn pause(&self) -> Result<()> {
        self.service
            .action(&self.url, "Pause", &pause_payload(self.instance_id))
            .await?;

        Ok(())
    }

    /// Stops the playback.
    pub async fn stop(&self) -> Result<()> {
        self.service
            .action(&self.url, "Stop", &stop_payload(self.instance_id))
            .await?;

        Ok(())
    }

    /// Seeks to a position in the media.
    pub async fn seek(&self, unit: SeekUnit, target: &str) -> Result<()> {
        let payload = seek_payload(self.instance_id, unit.as_str(), target);

        self.service.action(&self.url, "Seek", &payload).await?;

        Ok(())
    }

    /// Returns the current transport state of the renderer.
    pub async fn transport_info(&self) -> Result<TransportInfo> {
        let payload = format!("<InstanceID>{}</InstanceID>", self.instance_id);

        let response = self
            .service
            .action_ordered(&self.url, "GetTransportInfo", &payload)
            .await?;

        Ok(TransportInfo {
            current_transport_state: TransportState::from_name(
                response.get("CurrentTransportState").unwrap_or_default(),
            ),
            current_transport_status: response
                .get("CurrentTransportStatus")
                .unwrap_or_default()
                .to_string(),
            current_speed: response.get("CurrentSpeed").unwrap_or_default().to_string(),
        })
    }

    /// Returns the current transport state of the renderer.
    pub async fn state(&self) -> Result<TransportState> {
        Ok(self.transport_info().await?.current_transport_state)
    }

    /// Returns the position of the playback.
    pub async fn position_info(&self) -> Result<PositionInfo> {
        let payload = format!("<InstanceID>{}</InstanceID>", self.instance_id);

        let response = self
            .service
            .action_ordered(&self.url, "GetPositionInfo", &payload)
            .await?;

        Ok(PositionInfo {
            track: response.get_u32("Track").unwrap_or_default(),
            track_duration: response
                .get("TrackDuration")
                .unwrap_or_default()
                .to_string(),
            track_meta_data: response
                .get("TrackMetaData")
                .unwrap_or_default()
                .to_string(),
            track_uri: response.get("TrackURI").unwrap_or_default().to_string(),
            rel_time: response.get("RelTime").unwrap_or_default().to_string(),
            abs_time: response.get("AbsTime").unwrap_or_default().to_string(),
            rel_count: response.get_i32("RelCount").unwrap_or_default(),
            abs_count: response.get_i32("AbsCount").unwrap_or_default(),
        })
    }

    /// Waits until the renderer reaches the given state.
    ///
    /// The state is polled every 200 milliseconds. If the renderer does not reach the
    /// state within `timeout`, [`Error::StateNotReached`] is returned.
    pub async fn wait_for_state(
        &self,
        state: TransportState,
        timeout: Duration,
    ) -> Result<TransportState> {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let current = self.state().await?;

            if current == state {
                return Ok(current);
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(Error::StateNotReached(state.as_str().to_string()));
            }

            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

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
    use crate::didl::{Didl, DidlItem, DidlResource, class};
    use roxmltree::Document;

    fn parse(payload: &str) -> roxmltree::Document<'static> {
        let xml = format!("<root>{payload}</root>");
        // Leaks the xml string into the document, but the test only uses it right away.
        Document::parse(Box::leak(xml.into_boxed_str())).unwrap()
    }

    #[test]
    fn transport_state_is_parsed() {
        assert_eq!(
            TransportState::from_name("PLAYING"),
            TransportState::Playing
        );
        assert_eq!(
            TransportState::from_name("STREAMING"),
            TransportState::Playing
        );
        assert_eq!(
            TransportState::from_name("PAUSED_PLAYBACK"),
            TransportState::Paused
        );
        assert_eq!(
            TransportState::from_name("STOPPED"),
            TransportState::Stopped
        );
        assert_eq!(
            TransportState::from_name("STREAMING_BUFFERING"),
            TransportState::Buffering
        );
        assert_eq!(
            TransportState::from_name("NO_MEDIA_PRESENT"),
            TransportState::NoMedia
        );
        assert_eq!(
            TransportState::from_name("SOMETHING_ELSE"),
            TransportState::Unknown
        );
    }

    #[test]
    fn transport_state_round_trips() {
        for state in [
            TransportState::Idle,
            TransportState::Loading,
            TransportState::Playing,
            TransportState::Paused,
            TransportState::Stopped,
            TransportState::Transitioning,
            TransportState::Buffering,
            TransportState::NoMedia,
        ] {
            assert_eq!(TransportState::from_name(state.as_str()), state);
        }
    }

    #[test]
    fn playing_state_is_recognized() {
        assert!(TransportState::Playing.is_playing());
        assert!(!TransportState::Paused.is_playing());
        assert!(!TransportState::Stopped.is_playing());
    }

    #[test]
    fn seek_units_use_the_names_of_the_service() {
        assert_eq!(SeekUnit::Track.as_str(), "TRACK_NR");
        assert_eq!(SeekUnit::RelTime.as_str(), "REL_TIME");
        assert_eq!(SeekUnit::AbsTime.as_str(), "ABS_TIME");
    }

    #[test]
    fn durations_are_parsed() {
        assert_eq!(parse_duration("0:00:00.000"), Some(Duration::ZERO));
        assert_eq!(
            parse_duration("0:10:34.000"),
            Some(Duration::from_secs(10 * 60 + 34))
        );
        assert_eq!(
            parse_duration("1:02:03.500"),
            Some(Duration::from_secs(3600 + 2 * 60 + 3) + Duration::from_millis(500))
        );
        assert_eq!(
            parse_duration("02:03.000"),
            Some(Duration::from_secs(2 * 60 + 3))
        );
    }

    #[test]
    fn invalid_durations_are_rejected() {
        assert_eq!(parse_duration("NOT_IMPLEMENTED"), None);
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("1:2:3:4"), None);
        assert_eq!(parse_duration("abc"), None);
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

        assert!(
            doc.descendants()
                .all(|n| n.tag_name().name() != "CurrentURIMetaData")
        );
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
