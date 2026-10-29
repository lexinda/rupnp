use crate::{Error, Result};
use http::uri::PathAndQuery;
use http_body_util::BodyExt;
#[cfg(feature = "subscribe")]
use if_addrs::{get_if_addrs, IfAddr};
use roxmltree::{Document, Node};
use std::borrow::Cow;
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

/// Parses a URL endpoint of a service (e.g. `controlURL`) into a [`PathAndQuery`].
///
/// Some devices emit endpoints relative to the device description but without a leading
/// slash (e.g. `MediaRenderer/control`). [`PathAndQuery`] rejects such paths, so they are
/// normalized to an absolute path here.
pub fn parse_url_endpoint(node: Node<'_, '_>) -> Result<PathAndQuery> {
    let text = node.text().unwrap_or_default().trim();

    if text.is_empty() {
        return Err(Error::ParseError("empty url endpoint"));
    }

    let normalized = if text.starts_with('/') {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(format!("/{text}"))
    };

    normalized.parse().map_err(Error::invalid_response)
}

/// Parses an optional URL endpoint of a service (e.g. `eventSubURL`).
///
/// Returns `Ok(None)` if the element is not present in the device description.
pub fn parse_optional_url_endpoint(node: Option<Node<'_, '_>>) -> Result<Option<PathAndQuery>> {
    node.map(parse_url_endpoint).transpose()
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
    use crate::utils::parse_url_endpoint;
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
    fn test_parse_url_endpoint() -> Result<(), Box<dyn std::error::Error>> {
        let xml = r#"
        <service>
            <controlURL>/MediaRenderer/control</controlURL>
        </service>
        "#;

        let doc = Document::parse(xml).unwrap();
        let service_node = doc.root_element();

        #[allow(non_snake_case)]
        let control_url = find_in_xml! { service_node => controlURL };

        assert_eq!(
            parse_url_endpoint(control_url)?.as_str(),
            "/MediaRenderer/control"
        );

        Ok(())
    }

    #[test]
    fn test_parse_url_endpoint_without_leading_slash() -> Result<(), Box<dyn std::error::Error>> {
        let xml = r#"
        <service>
            <controlURL>MediaRenderer/control</controlURL>
        </service>
        "#;

        let doc = Document::parse(xml).unwrap();
        let service_node = doc.root_element();

        #[allow(non_snake_case)]
        let control_url = find_in_xml! { service_node => controlURL };

        assert_eq!(
            parse_url_endpoint(control_url)?.as_str(),
            "/MediaRenderer/control"
        );

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
