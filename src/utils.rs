use crate::{Error, Result};
use http::Uri;
use http_body_util::BodyExt;
#[cfg(feature = "subscribe")]
use if_addrs::{get_if_addrs, IfAddr};
use roxmltree::{Document, Node};
#[cfg(feature = "subscribe")]
use std::net::{IpAddr, SocketAddrV4};
use std::time::Duration;

pub(crate) trait HttpResponseExt: Sized {
    fn err_if_not_200(self) -> Result<Self>;
}
impl<B> HttpResponseExt for hyper::Response<B> {
    fn err_if_not_200(self) -> Result<Self> {
        if self.status() != 200 {
            Err(Error::HttpErrorCode(self.status()))
        } else {
            Ok(self)
        }
    }
}
pub(crate) trait HyperBodyExt: Sized {
    async fn bytes(self) -> Result<bytes::Bytes>;
}
impl HyperBodyExt for hyper::body::Incoming {
    async fn bytes(self) -> Result<bytes::Bytes> {
        Ok(self.collect().await?.to_bytes())
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! find_in_xml {
    ( $node:expr => $( $($var:ident)? $(?$var_opt:ident)? ),+ $(#$var_hash_opt:ident)? ) => { {
        let node = $node;
        $(
            $(let mut $var = None;)?
            $(let mut $var_opt = None;)?
        )*
        $(
            let mut $var_hash_opt: HashMap<String, Option<String>> = $var_hash_opt
                .iter()
                .map(|k| (k.to_string(), None))
                .collect();
        )?
        for child in node.children().filter(roxmltree::Node::is_element) {
            match child.tag_name().name() {
                $(
                    $(stringify!($var) => $var = Some(child),)?
                    $(stringify!($var_opt) => $var_opt = Some(child),)?
                )*
                _ => (),
            }
            $(
                if $var_hash_opt.contains_key(child.tag_name().name()) {
                    $var_hash_opt.insert(
                        child.tag_name().name().to_string(),
                        $crate::utils::parse_node_text(child).ok());
                }
            )?
        }

        $($(
            let $var = $var.ok_or_else(|| $crate::Error::XmlMissingElement(
                node.tag_name().name().to_string(),
                stringify!($var).to_string(),
            ))?;
        )?)*

        (
            $(
                $($var)?
                $($var_opt)?
            ),*
            $( $var_hash_opt )?
        )
    } }
}

pub fn parse_node_text<T, E>(node: Node<'_, '_>) -> Result<T>
where
    T: std::str::FromStr<Err = E>,
    E: std::error::Error + Send + Sync + 'static,
{
    node.text()
        .unwrap_or_default()
        .parse()
        .map_err(Error::invalid_response)
}

/// Returns the text of an element, trimmed.
pub fn node_text(node: Node<'_, '_>) -> String {
    node.text().unwrap_or_default().trim().to_string()
}

/// Returns the first child element with the given name.
pub fn find_child<'a, 'input: 'a>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .filter(Node::is_element)
        .find(|child| child.tag_name().name().eq_ignore_ascii_case(name))
}

/// Parses a URL endpoint of a service (e.g. `controlURL`) into a url reference.
///
/// The reference is not resolved here, it is resolved against the base url of the device
/// description when it is used.
pub fn parse_endpoint(node: Node<'_, '_>) -> Result<String> {
    let reference = node_text(node);

    validate_reference(&reference)?;

    Ok(reference)
}

/// Parses an optional URL endpoint of a service (e.g. `eventSubURL`).
///
/// Returns `Ok(None)` if the element is not present in the device description.
pub fn parse_optional_endpoint(node: Option<Node<'_, '_>>) -> Result<Option<String>> {
    node.map(parse_endpoint).transpose()
}

/// Checks that a string is a usable url reference.
fn validate_reference(reference: &str) -> Result<()> {
    if reference.is_empty() {
        return Err(Error::ParseError("empty url endpoint"));
    }

    if reference
        .chars()
        .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(Error::ParseError(
            "url endpoint contains invalid characters",
        ));
    }

    if has_scheme(reference) {
        reference.parse::<Uri>().map_err(Error::invalid_response)?;
    }

    Ok(())
}

/// Returns the base url of the document at the given url.
///
/// The base url is the url of the document without its last path segment, so
/// `http://host:1400/xml/device_description.xml` has the base url
/// `http://host:1400/xml/`.
pub fn base_url(url: &Uri) -> Uri {
    let path = url.path();

    let dir = match path.rfind('/') {
        Some(idx) => &path[..=idx],
        None => "/",
    };

    let mut parts = url.clone().into_parts();
    parts.path_and_query = Some(dir.parse().expect("infallible"));

    Uri::from_parts(parts).expect("infallible")
}

/// Resolves a url reference against a base url.
///
/// The resolution follows RFC 3986:
///
/// * a reference with a scheme is used as it is
/// * a root relative reference (`/foo`) keeps the scheme and authority of the base url
/// * a relative reference (`foo`) is resolved against the path of the base url
///
/// Dot segments (`..` and `.`) are removed from the result.
pub fn resolve_url(base: &Uri, reference: &str) -> Result<Uri> {
    let reference = reference.trim();

    if reference.is_empty() {
        return Err(Error::ParseError("empty url"));
    }

    if has_scheme(reference) {
        return reference.parse().map_err(Error::invalid_response);
    }

    // The fragment is not part of the request, so it is dropped.
    let reference = reference.split('#').next().unwrap_or_default();

    let (path, query) = match reference.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (reference, None),
    };

    let base_path = base.path();

    let merged = if path.is_empty() {
        base_path.to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        // The base path up to and including its last slash.
        let dir = match base_path.rfind('/') {
            Some(idx) => &base_path[..=idx],
            None => "/",
        };

        format!("{dir}{path}")
    };

    let merged = remove_dot_segments(&merged);

    let mut path_and_query = merged;
    if let Some(query) = query {
        path_and_query.push('?');
        path_and_query.push_str(query);
    }

    let mut parts = base.clone().into_parts();
    parts.path_and_query = Some(path_and_query.parse().map_err(Error::invalid_response)?);

    Uri::from_parts(parts).map_err(Error::invalid_response)
}

/// Returns whether a url reference has a scheme.
fn has_scheme(reference: &str) -> bool {
    let Some((scheme, _)) = reference.split_once(':') else {
        return false;
    };

    let mut chars = scheme.chars();

    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }

    chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

/// Removes the dot segments of a path, as described in RFC 3986.
fn remove_dot_segments(path: &str) -> String {
    let mut input = path.to_string();
    let mut output = String::new();

    while !input.is_empty() {
        if let Some(rest) = input.strip_prefix("../") {
            input = rest.to_string();
        } else if let Some(rest) = input.strip_prefix("./") {
            input = rest.to_string();
        } else if input.starts_with("/./") {
            input.replace_range(..3, "/");
        } else if input == "/." {
            input = "/".to_string();
        } else if input.starts_with("/../") {
            input.replace_range(..4, "/");
            remove_last_segment(&mut output);
        } else if input == "/.." {
            input = "/".to_string();
            remove_last_segment(&mut output);
        } else if input == "." || input == ".." {
            input.clear();
        } else {
            // Move the first path segment to the output.
            let start = usize::from(input.starts_with('/'));
            let end = match input[start..].find('/') {
                Some(idx) => start + idx,
                None => input.len(),
            };

            output.push_str(&input[..end]);
            input.drain(..end);
        }
    }

    output
}

/// Removes the last segment of a path.
fn remove_last_segment(path: &mut String) {
    match path.rfind('/') {
        Some(idx) => path.truncate(idx),
        None => path.clear(),
    }
}

pub fn find_root<'a, 'input: 'a>(
    document: &'input Document<'_>,
    element: &str,
    docname: &str,
) -> Result<Node<'a, 'input>> {
    document
        .descendants()
        .filter(Node::is_element)
        .find(|n| n.tag_name().name().eq_ignore_ascii_case(element))
        .ok_or_else(|| Error::XmlMissingElement(docname.to_string(), element.to_string()))
}

pub fn find_node_attribute<'n, 'd: 'n>(node: Node<'d, 'n>, attr: &str) -> Option<&'n str> {
    node.attributes()
        .find(|a| a.name().eq_ignore_ascii_case(attr))
        .map(|a| a.value())
}

#[cfg(feature = "subscribe")]
/// Returns the local addresses that can be used to reach devices on the local network.
///
/// Loopback addresses are skipped.
pub fn get_local_addrs() -> Result<Vec<IpAddr>> {
    Ok(local_ifaces()?.into_iter().map(|(ip, _)| ip).collect())
}

/// Returns the local addresses together with the netmask of the interface they belong to.
#[cfg(feature = "subscribe")]
fn local_ifaces() -> Result<Vec<(IpAddr, IpAddr)>> {
    Ok(get_if_addrs()?
        .iter()
        .map(|iface| (iface.ip(), netmask_of(&iface.addr)))
        .filter(|(ip, _)| !ip.is_loopback())
        .collect())
}

/// Returns the netmask of an interface address.
#[cfg(feature = "subscribe")]
fn netmask_of(addr: &IfAddr) -> IpAddr {
    match addr {
        IfAddr::V4(addr) => IpAddr::V4(addr.netmask),
        IfAddr::V6(addr) => IpAddr::V6(addr.netmask),
    }
}

/// Returns whether the address belongs to a private network.
#[cfg(feature = "subscribe")]
fn is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private(),
        // Unique local addresses (fc00::/7). `Ipv6Addr::is_unique_local` is only
        // available since Rust 1.84, which is newer than our MSRV.
        IpAddr::V6(ip) => (ip.octets()[0] & 0xfe) == 0xfc,
    }
}

#[cfg(feature = "subscribe")]
/// Picks the local address that is most likely reachable from `peer`.
///
/// If `peer` is on the same subnet as one of the local interfaces, the address of that
/// interface is returned. Otherwise the first private address is used. This avoids
/// picking the wrong interface on machines with multiple network adapters (for example
/// a VPN or a virtual machine adapter).
pub fn pick_local_addr(peer: Option<IpAddr>) -> Result<IpAddr> {
    let ifaces = local_ifaces()?;

    if ifaces.is_empty() {
        return Err(Error::NoLocalInterfaceOpen);
    }

    if let Some(peer) = peer {
        if let Some((ip, _)) = ifaces
            .iter()
            .find(|(ip, netmask)| same_subnet(ip, netmask, &peer))
        {
            return Ok(*ip);
        }
    }

    if let Some((ip, _)) = ifaces.iter().find(|(ip, _)| is_private(ip)) {
        return Ok(*ip);
    }

    Ok(ifaces[0].0)
}

/// Returns whether `ip` and `peer` are located in the same subnet.
#[cfg(feature = "subscribe")]
fn same_subnet(ip: &IpAddr, netmask: &IpAddr, peer: &IpAddr) -> bool {
    match (ip, netmask, peer) {
        (IpAddr::V4(ip), IpAddr::V4(netmask), IpAddr::V4(peer)) => {
            u32::from(*ip) & u32::from(*netmask) == u32::from(*peer) & u32::from(*netmask)
        }
        (IpAddr::V6(ip), IpAddr::V6(netmask), IpAddr::V6(peer)) => {
            u128::from(*ip) & u128::from(*netmask) == u128::from(*peer) & u128::from(*netmask)
        }
        _ => false,
    }
}

#[cfg(feature = "subscribe")]
/// Returns the IP address of the peer of `url`, if it is given as a literal address.
pub fn peer_ip(url: &http::Uri) -> Option<IpAddr> {
    let host = url.host()?;
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    host.parse().ok()
}

#[cfg(feature = "subscribe")]
/// Returns a local IPv4 address and the port 0.
///
/// Kept for backwards compatibility, prefer [`pick_local_addr`] which is able to take the
/// address of the peer into account.
pub fn get_local_addr() -> Result<SocketAddrV4> {
    let ifaces = local_ifaces()?;

    let ip = ifaces
        .iter()
        .find(|(ip, _)| matches!(ip, IpAddr::V4(_)) && is_private(ip))
        .or_else(|| ifaces.iter().find(|(ip, _)| matches!(ip, IpAddr::V4(_))))
        .map(|(ip, _)| *ip)
        .ok_or(Error::NoLocalInterfaceOpen)?;

    match ip {
        IpAddr::V4(ip) => Ok(SocketAddrV4::new(ip, 0)),
        IpAddr::V6(_) => Err(Error::NoLocalInterfaceOpen),
    }
}

/// Runs `fut`, giving up after `timeout`.
///
/// Requires a tokio runtime with the time driver enabled.
pub(crate) async fn with_timeout<F, T>(timeout: Duration, fut: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    match tokio::time::timeout(timeout, fut).await {
        Ok(result) => result,
        Err(_) => Err(Error::Timeout),
    }
}

#[cfg(test)]
mod tests {
    use crate::utils::{base_url, parse_endpoint, resolve_url};
    use http::Uri;
    use roxmltree::Document;
    use std::collections::HashMap;
    use std::time::Duration;

    #[cfg(feature = "subscribe")]
    use std::net::IpAddr;

    #[tokio::test]
    async fn timeout_is_enforced() {
        let result = crate::utils::with_timeout(Duration::from_millis(50), async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok::<_, crate::Error>(())
        })
        .await;

        assert!(matches!(result, Err(crate::Error::Timeout)));
    }

    #[cfg(feature = "subscribe")]
    mod subscribe {
        use super::*;
        use crate::utils::{peer_ip, same_subnet};
        use std::net::Ipv4Addr;

        #[test]
        fn same_subnet_matches_the_network_part() {
            let netmask = IpAddr::V4(Ipv4Addr::new(255, 255, 255, 0));
            let local = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));
            let peer_same = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 55));
            let peer_other = IpAddr::V4(Ipv4Addr::new(192, 168, 2, 55));

            assert!(same_subnet(&local, &netmask, &peer_same));
            assert!(!same_subnet(&local, &netmask, &peer_other));
        }

        #[test]
        fn same_subnet_does_not_mix_ip_versions() {
            let local = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));
            let netmask = IpAddr::V4(Ipv4Addr::new(255, 255, 255, 0));
            let peer = IpAddr::V6(std::net::Ipv6Addr::LOCALHOST);

            assert!(!same_subnet(&local, &netmask, &peer));
        }

        #[test]
        fn peer_ip_parses_literal_addresses() {
            let url: http::Uri = "http://192.168.1.5:1400/xml/device_description.xml"
                .parse()
                .unwrap();
            assert_eq!(peer_ip(&url), Some("192.168.1.5".parse().unwrap()));

            let url: http::Uri = "http://[fe80::1]:1400/xml/device_description.xml"
                .parse()
                .unwrap();
            assert_eq!(peer_ip(&url), Some("fe80::1".parse().unwrap()));

            let url: http::Uri = "http://example.local/xml/device_description.xml"
                .parse()
                .unwrap();
            assert_eq!(peer_ip(&url), None);
        }
    }

    #[test]
    fn base_url_is_the_url_without_its_last_segment() {
        let url: Uri = "http://192.168.1.5:1400/xml/device_description.xml"
            .parse()
            .unwrap();
        assert_eq!(base_url(&url).to_string(), "http://192.168.1.5:1400/xml/");

        let url: Uri = "http://192.168.1.5:1400/device_description.xml"
            .parse()
            .unwrap();
        assert_eq!(base_url(&url).to_string(), "http://192.168.1.5:1400/");

        let url: Uri = "http://192.168.1.5:1400".parse().unwrap();
        assert_eq!(base_url(&url).to_string(), "http://192.168.1.5:1400/");
    }

    #[test]
    fn relative_urls_are_resolved_against_the_base() {
        let base: Uri = "http://192.168.1.5:1400/xml/".parse().unwrap();

        assert_eq!(
            resolve_url(&base, "MediaRenderer/control")
                .unwrap()
                .to_string(),
            "http://192.168.1.5:1400/xml/MediaRenderer/control"
        );
        assert_eq!(
            resolve_url(&base, "./MediaRenderer/control")
                .unwrap()
                .to_string(),
            "http://192.168.1.5:1400/xml/MediaRenderer/control"
        );
    }

    #[test]
    fn root_relative_urls_keep_the_authority_of_the_base() {
        let base: Uri = "http://192.168.1.5:1400/xml/".parse().unwrap();

        assert_eq!(
            resolve_url(&base, "/MediaRenderer/control")
                .unwrap()
                .to_string(),
            "http://192.168.1.5:1400/MediaRenderer/control"
        );
    }

    #[test]
    fn absolute_urls_are_used_as_they_are() {
        let base: Uri = "http://192.168.1.5:1400/xml/".parse().unwrap();

        assert_eq!(
            resolve_url(&base, "http://192.168.1.9:1400/control")
                .unwrap()
                .to_string(),
            "http://192.168.1.9:1400/control"
        );
    }

    #[test]
    fn dot_segments_are_removed() {
        let base: Uri = "http://192.168.1.5:1400/xml/".parse().unwrap();

        assert_eq!(
            resolve_url(&base, "../control").unwrap().to_string(),
            "http://192.168.1.5:1400/control"
        );
        assert_eq!(
            resolve_url(&base, "a/../control").unwrap().to_string(),
            "http://192.168.1.5:1400/xml/control"
        );
        assert_eq!(
            resolve_url(&base, "./a/./b").unwrap().to_string(),
            "http://192.168.1.5:1400/xml/a/b"
        );
    }

    #[test]
    fn query_strings_are_kept() {
        let base: Uri = "http://192.168.1.5:1400/xml/".parse().unwrap();

        assert_eq!(
            resolve_url(&base, "control?id=1").unwrap().to_string(),
            "http://192.168.1.5:1400/xml/control?id=1"
        );
    }

    #[test]
    fn empty_references_are_rejected() {
        let base: Uri = "http://192.168.1.5:1400/xml/".parse().unwrap();

        assert!(resolve_url(&base, "").is_err());
        assert!(resolve_url(&base, "   ").is_err());
    }

    #[test]
    fn endpoints_are_validated() -> Result<(), Box<dyn std::error::Error>> {
        let xml = r#"
        <service>
            <controlURL>/MediaRenderer/control</controlURL>
        </service>
        "#;

        let doc = Document::parse(xml).unwrap();
        let service_node = doc.root_element();

        #[allow(non_snake_case)]
        let control_url = find_in_xml! { service_node => controlURL };

        assert_eq!(parse_endpoint(control_url)?, "/MediaRenderer/control");

        Ok(())
    }

    #[test]
    fn endpoints_with_invalid_characters_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let xml = r#"
        <service>
            <controlURL>Media Renderer/control</controlURL>
        </service>
        "#;

        let doc = Document::parse(xml).unwrap();
        let service_node = doc.root_element();

        #[allow(non_snake_case)]
        let control_url = find_in_xml! { service_node => controlURL };

        assert!(parse_endpoint(control_url).is_err());

        Ok(())
    }

    #[test]
    fn test_find_in_xml_macro() -> Result<(), Box<dyn std::error::Error>> {
        let xml = r#"
        <device>
            <deviceType>urn:schemas-upnp-org:device:MediaServer:1</deviceType>
            <friendlyName>My Media Server</friendlyName>
            <manufacturer>ACME Corp</manufacturer>
            <modelName>MediaBox 3000</modelName>
            <serviceList>
                <service>
                    <serviceType>urn:schemas-upnp-org:service:ContentDirectory:1</serviceType>
                </service>
            </serviceList>
            <deviceList>
                <device>
                    <deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
                </device>
            </deviceList>
            <extraInfo>Some extra information</extraInfo>
        </device>
        "#;

        let doc = Document::parse(xml).unwrap();
        let device_node = doc.root_element();

        let extra_element_keys = &["extraInfo"];

        #[rustfmt::skip]
        #[allow(non_snake_case)]
        let (device_type, friendly_name, services, devices, extra_elements) = 
            find_in_xml! { device_node => deviceType, friendlyName, ?serviceList, ?deviceList, #extra_element_keys };

        // Test required elements
        assert_eq!(
            device_type.text(),
            Some("urn:schemas-upnp-org:device:MediaServer:1")
        );
        assert_eq!(friendly_name.text(), Some("My Media Server"));

        // Test optional elements
        assert!(services.is_some());
        assert!(devices.is_some());

        // Test extra elements
        let mut expected_extra = HashMap::new();
        expected_extra.insert(
            "extraInfo".to_string(),
            Some("Some extra information".to_string()),
        );
        assert_eq!(extra_elements, expected_extra);

        // Test non-existent optional element
        let extra_element_keys = &["nonExistentElement"];

        #[rustfmt::skip]
        #[allow(non_snake_case)]
        let (_, _, _, _, missing_extra) =
            find_in_xml! { device_node => deviceType, friendlyName, ?serviceList, ?deviceList, #extra_element_keys };

        let mut expected_missing = HashMap::new();
        expected_missing.insert("nonExistentElement".to_string(), None);
        assert_eq!(missing_extra, expected_missing);

        Ok(())
    }
}
