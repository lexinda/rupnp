//! Builder for DIDL-Lite metadata.
//!
//! DIDL-Lite is the metadata format used by the UPnP AVTransport and ContentDirectory
//! services to describe a media item. A renderer needs it to show the title of the media
//! it is about to play.
//!
//! # Example usage:
//! ```rust
//! use rupnp::didl::{class, Didl, DidlItem, DidlResource};
//!
//! let didl = Didl::new()
//!     .item(
//!         DidlItem::new()
//!             .id("1")
//!             .title("Big Buck Bunny")
//!             .class(class::VIDEO)
//!             .resource(
//!                 DidlResource::new(
//!                     "http-get:*:video/mp4:*",
//!                     "http://192.168.1.10:8080/bbb.mp4",
//!                 )
//!                 .size(12_345_678),
//!             ),
//!     )
//!     .to_xml();
//!
//! assert!(didl.contains("Big Buck Bunny"));
//! ```

use crate::xml::{escape_attr, escape_text};

/// Namespace of the DIDL-Lite elements.
pub const NS_DIDL_LITE: &str = "urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/";
/// Namespace of the Dublin Core elements.
pub const NS_DC: &str = "http://purl.org/dc/elements/1.1/";
/// Namespace of the UPnP elements.
pub const NS_UPNP: &str = "urn:schemas-upnp-org:metadata-1-0/upnp/";
/// Namespace of the DLNA elements.
pub const NS_DLNA: &str = "urn:schemas-upnp-org:metadata-1-0/dlna/";
/// Namespace of the `res` elements.
pub const NS_RES: &str = "http://www.dlna.org/soap/";
/// Namespace of the Samsung elements.
pub const NS_SEC: &str = "http://www.sec.co.kr/";

/// The `upnp:class` of the different kinds of media.
pub mod class {
    /// A video file.
    pub const VIDEO: &str = "object.item.videoItem";
    /// A video file that is a recording.
    pub const VIDEO_RECORDING: &str = "object.item.videoItem.videoRecording";
    /// An audio track.
    pub const AUDIO: &str = "object.item.audioItem.musicTrack";
    /// An audio track that is a recording.
    pub const AUDIO_RECORDING: &str = "object.item.audioItem.audioRecording";
    /// A photo.
    pub const IMAGE: &str = "object.item.imageItem.photo";
}

/// A namespace an element can be placed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    /// The Dublin Core namespace.
    Dc,
    /// The UPnP namespace.
    Upnp,
    /// The DLNA namespace.
    Dlna,
    /// The Samsung namespace.
    Sec,
}

impl Namespace {
    /// Returns the prefix the namespace is declared with.
    pub fn prefix(self) -> &'static str {
        match self {
            Namespace::Dc => "dc",
            Namespace::Upnp => "upnp",
            Namespace::Dlna => "dlna",
            Namespace::Sec => "sec",
        }
    }

    /// Returns the uri of the namespace.
    pub fn uri(self) -> &'static str {
        match self {
            Namespace::Dc => NS_DC,
            Namespace::Upnp => NS_UPNP,
            Namespace::Dlna => NS_DLNA,
            Namespace::Sec => NS_SEC,
        }
    }
}

/// A generic element of a DIDL-Lite item.
#[derive(Debug, Clone)]
pub struct DidlElement {
    namespace: Namespace,
    name: String,
    value: String,
}

impl DidlElement {
    /// Creates a new element in the given namespace.
    pub fn new(namespace: Namespace, name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            namespace,
            name: name.into(),
            value: value.into(),
        }
    }
}

/// A resource of a DIDL-Lite item, pointing at the actual media file.
#[derive(Debug, Clone)]
pub struct DidlResource {
    protocol_info: String,
    uri: String,
    attributes: Vec<(String, String)>,
}

impl DidlResource {
    /// Creates a new resource.
    ///
    /// `protocol_info` describes the format of the media, for example
    /// `http-get:*:video/mp4:*`. `uri` points at the media file.
    pub fn new(protocol_info: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            protocol_info: protocol_info.into(),
            uri: uri.into(),
            attributes: Vec::new(),
        }
    }

    /// Sets the duration of the media, formatted as `H:MM:SS.mmm`.
    pub fn duration(mut self, duration: impl Into<String>) -> Self {
        self.attributes
            .push(("duration".to_string(), duration.into()));
        self
    }

    /// Sets the size of the media in bytes.
    pub fn size(mut self, size: u64) -> Self {
        self.attributes.push(("size".to_string(), size.to_string()));
        self
    }

    /// Sets the resolution of the media, for example `1920x1080`.
    pub fn resolution(mut self, resolution: impl Into<String>) -> Self {
        self.attributes
            .push(("resolution".to_string(), resolution.into()));
        self
    }

    /// Sets the bitrate of the media in bits per second.
    pub fn bitrate(mut self, bitrate: u32) -> Self {
        self.attributes
            .push(("bitrate".to_string(), bitrate.to_string()));
        self
    }

    /// Sets an arbitrary attribute of the resource.
    pub fn attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.push((name.into(), value.into()));
        self
    }
}

/// An item of a DIDL-Lite document.
#[derive(Debug, Clone)]
pub struct DidlItem {
    id: String,
    parent_id: String,
    restricted: bool,
    title: String,
    class: String,
    creator: Option<String>,
    album_art_uri: Option<String>,
    resources: Vec<DidlResource>,
    elements: Vec<DidlElement>,
}

impl Default for DidlItem {
    fn default() -> Self {
        Self {
            id: "0".to_string(),
            parent_id: "-1".to_string(),
            restricted: true,
            title: String::new(),
            class: String::new(),
            creator: None,
            album_art_uri: None,
            resources: Vec::new(),
            elements: Vec::new(),
        }
    }
}

impl DidlItem {
    /// Creates a new item.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the id of the item.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    /// Sets the id of the parent item.
    pub fn parent_id(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = parent_id.into();
        self
    }

    /// Sets whether the item is restricted.
    pub fn restricted(mut self, restricted: bool) -> Self {
        self.restricted = restricted;
        self
    }

    /// Sets the title of the item.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Sets the `upnp:class` of the item, see the [`class`] module.
    pub fn class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }

    /// Sets the creator of the item.
    pub fn creator(mut self, creator: impl Into<String>) -> Self {
        self.creator = Some(creator.into());
        self
    }

    /// Sets the album art of the item.
    pub fn album_art_uri(mut self, uri: impl Into<String>) -> Self {
        self.album_art_uri = Some(uri.into());
        self
    }

    /// Adds a resource to the item.
    pub fn resource(mut self, resource: DidlResource) -> Self {
        self.resources.push(resource);
        self
    }

    /// Adds an arbitrary element to the item.
    pub fn element(mut self, element: DidlElement) -> Self {
        self.elements.push(element);
        self
    }
}

/// A DIDL-Lite document.
#[derive(Debug, Default, Clone)]
pub struct Didl {
    items: Vec<DidlItem>,
}

impl Didl {
    /// Creates an empty document.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an item to the document.
    pub fn item(mut self, item: DidlItem) -> Self {
        self.items.push(item);
        self
    }

    /// Serializes the document into a DIDL-Lite xml fragment.
    ///
    /// The result does not contain an xml declaration, so that it can be embedded into
    /// another document. Use [`to_xml_document`](Didl::to_xml_document) to get a standalone
    /// document.
    pub fn to_xml(&self) -> String {
        let mut xml = String::new();

        xml.push_str("<DIDL-Lite");
        for (prefix, uri) in [
            ("dc", NS_DC),
            ("upnp", NS_UPNP),
            ("dlna", NS_DLNA),
            ("sec", NS_SEC),
        ] {
            xml.push_str(&format!(" xmlns:{prefix}=\"{uri}\""));
        }
        xml.push_str(&format!(" xmlns=\"{NS_DIDL_LITE}\">"));

        for item in &self.items {
            item.write_to(&mut xml);
        }

        xml.push_str("</DIDL-Lite>");

        xml
    }

    /// Serializes the document into a standalone DIDL-Lite xml document.
    pub fn to_xml_document(&self) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>{}",
            self.to_xml()
        )
    }
}

impl DidlItem {
    fn write_to(&self, xml: &mut String) {
        xml.push_str(&format!(
            "<item id=\"{}\" parentID=\"{}\" restricted=\"{}\">",
            escape_attr(&self.id),
            escape_attr(&self.parent_id),
            if self.restricted { "1" } else { "0" },
        ));

        xml.push_str(&format!(
            "<dc:title>{}</dc:title>",
            escape_text(&self.title)
        ));

        if !self.class.is_empty() {
            xml.push_str(&format!(
                "<upnp:class>{}</upnp:class>",
                escape_text(&self.class)
            ));
        }

        if let Some(creator) = &self.creator {
            xml.push_str(&format!(
                "<dc:creator>{}</dc:creator>",
                escape_text(creator)
            ));
        }

        for element in &self.elements {
            xml.push_str(&format!(
                "<{prefix}:{name}>{value}</{prefix}:{name}>",
                prefix = element.namespace.prefix(),
                name = escape_attr(&element.name),
                value = escape_text(&element.value),
            ));
        }

        if let Some(uri) = &self.album_art_uri {
            xml.push_str(&format!(
                "<upnp:albumArtURI>{}</upnp:albumArtURI>",
                escape_text(uri)
            ));
        }

        for resource in &self.resources {
            xml.push_str(&format!("<res xmlns=\"{NS_RES}\""));
            xml.push_str(&format!(
                " protocolInfo=\"{}\"",
                escape_attr(&resource.protocol_info)
            ));
            for (name, value) in &resource.attributes {
                xml.push_str(&format!(
                    " {name}=\"{value}\"",
                    name = escape_attr(name),
                    value = escape_attr(value),
                ));
            }
            xml.push_str(&format!(">{}</res>", escape_text(&resource.uri)));
        }

        xml.push_str("</item>");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roxmltree::Document;

    fn sample() -> Didl {
        Didl::new().item(
            DidlItem::new()
                .id("1")
                .title("Big Buck Bunny")
                .class(class::VIDEO)
                .creator("Blender Foundation")
                .album_art_uri("http://192.168.1.10:8080/cover.jpg")
                .resource(
                    DidlResource::new("http-get:*:video/mp4:*", "http://192.168.1.10:8080/bbb.mp4")
                        .size(12_345_678)
                        .duration("0:10:34.000"),
                ),
        )
    }

    #[test]
    fn document_is_valid_xml() {
        let xml = sample().to_xml_document();
        let doc = Document::parse(&xml).unwrap();

        assert_eq!(doc.root_element().tag_name().name(), "DIDL-Lite");
    }

    #[test]
    fn item_contains_the_title_and_class() {
        let xml = sample().to_xml();
        let doc = Document::parse(&xml).unwrap();

        let item = doc.root_element().first_element_child().unwrap();
        assert_eq!(item.tag_name().name(), "item");
        assert_eq!(item.attribute("id"), Some("1"));
        assert_eq!(item.attribute("parentID"), Some("-1"));
        assert_eq!(item.attribute("restricted"), Some("1"));

        let title = item
            .children()
            .find(|c| c.tag_name().name() == "title")
            .unwrap();
        assert_eq!(title.text(), Some("Big Buck Bunny"));

        let class = item
            .children()
            .find(|c| c.tag_name().name() == "class")
            .unwrap();
        assert_eq!(class.text(), Some(class::VIDEO));
    }

    #[test]
    fn resource_contains_the_protocol_info_and_the_uri() {
        let xml = sample().to_xml();
        let doc = Document::parse(&xml).unwrap();

        let res = doc
            .descendants()
            .find(|n| n.tag_name().name() == "res")
            .unwrap();

        assert_eq!(
            res.attribute("protocolInfo"),
            Some("http-get:*:video/mp4:*")
        );
        assert_eq!(res.attribute("size"), Some("12345678"));
        assert_eq!(res.attribute("duration"), Some("0:10:34.000"));
        assert_eq!(res.text(), Some("http://192.168.1.10:8080/bbb.mp4"));
    }

    #[test]
    fn special_characters_are_escaped() {
        let xml =
            Didl::new()
                .item(DidlItem::new().title("Fish & Chips <b>bold</b>").resource(
                    DidlResource::new(
                        "http-get:*:video/mp4:*",
                        "http://192.168.1.10:8080/a.mp4?b=1&c=2",
                    ),
                ))
                .to_xml();

        // The document has to stay valid xml.
        let doc = Document::parse(&xml).unwrap();

        let title = doc
            .descendants()
            .find(|n| n.tag_name().name() == "title")
            .unwrap();
        assert_eq!(title.text(), Some("Fish & Chips <b>bold</b>"));

        let res = doc
            .descendants()
            .find(|n| n.tag_name().name() == "res")
            .unwrap();
        assert_eq!(res.text(), Some("http://192.168.1.10:8080/a.mp4?b=1&c=2"));
    }
}
