use crate::{
    error::{Error, UPnPError},
    find_in_xml,
    scpd::SCPD,
    utils::{self, HttpResponseExt},
    Result,
};

use bytes::Bytes;
#[cfg(feature = "subscribe")]
use futures_core::stream::Stream;
#[cfg(feature = "subscribe")]
use genawaiter::sync::{Co, Gen};
use http_body_util::Empty;
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use log::debug;
#[cfg(feature = "subscribe")]
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

use http::{uri::PathAndQuery, Request, Uri};
use roxmltree::{Document, Node};
use ssdp_client::URN;

use std::collections::HashMap;
#[cfg(feature = "subscribe")]
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use utils::HyperBodyExt;

/// A UPnP Service is the description of endpoints on a device for performing actions and reading
/// the service definition.
/// For a list of actions and state variables the service provides, take a look at [`scpd`](struct.Service.html#method.scpd).
#[derive(Debug, Clone)]
pub struct Service {
    service_type: URN,
    service_id: String,
    scpd_endpoint: PathAndQuery,
    control_endpoint: PathAndQuery,
    event_sub_endpoint: Option<PathAndQuery>,
}

impl Service {
    pub(crate) fn from_xml(node: Node<'_, '_>) -> Result<Self> {
        #[allow(non_snake_case)]
        let (service_type, service_id, scpd_endpoint, control_endpoint, event_sub_endpoint) =
            find_in_xml! { node => serviceType, serviceId, SCPDURL, controlURL, ?eventSubURL };

        Ok(Self {
            service_type: utils::parse_node_text(service_type)?,
            service_id: utils::parse_node_text(service_id)?,
            scpd_endpoint: utils::parse_url_endpoint(scpd_endpoint)?,
            control_endpoint: utils::parse_url_endpoint(control_endpoint)?,
            event_sub_endpoint: utils::parse_optional_url_endpoint(event_sub_endpoint)?,
        })
    }

    /// Returns the [URN](ssdp_client::URN) of this service.
    pub fn service_type(&self) -> &URN {
        &self.service_type
    }

    /// Returns the `Service Identifier`.
    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    pub(crate) fn control_url(&self, url: &Uri) -> Uri {
        replace_url_path(url, &self.control_endpoint)
    }
    pub(crate) fn scpd_url(&self, url: &Uri) -> Uri {
        replace_url_path(url, &self.scpd_endpoint)
    }
    pub(crate) fn event_sub_url(&self, url: &Uri) -> Option<Uri> {
        self.event_sub_endpoint
            .as_ref()
            .map(|path| replace_url_path(url, path))
    }

    /// Returns whether this service provides an `eventSubURL`.
    ///
    /// Services without an `eventSubURL` cannot be subscribed to: subscribing to them, or
    /// calling [`renew_subscription`](Service::renew_subscription) or
    /// [`unsubscribe`](Service::unsubscribe) on them, fails with
    /// [`Error::EventSubscriptionNotSupported`](crate::Error::EventSubscriptionNotSupported).
    pub fn supports_events(&self) -> bool {
        self.event_sub_endpoint.is_some()
    }

    /// Fetches the [`SCPD`](scpd/struct.SCPD.html) of this service.
    pub async fn scpd(&self, url: &Uri) -> Result<SCPD> {
        SCPD::from_url(
            &self.scpd_url(url),
            self.service_type().clone(),
            crate::DEFAULT_TIMEOUT,
        )
        .await
    }

    /// Fetches the [`SCPD`](scpd/struct.SCPD.html) of this service, giving up after
    /// `timeout`.
    pub async fn scpd_with_timeout(&self, url: &Uri, timeout: Duration) -> Result<SCPD> {
        SCPD::from_url(&self.scpd_url(url), self.service_type().clone(), timeout).await
    }

    /// Execute some UPnP Action on this service.
    /// The URL is usually obtained by the device this service was found on.
    /// The payload is xml-formatted data.
    ///
    /// # Example usage:
    ///
    /// ```rust,no_run
    /// # use ssdp_client::URN;
    /// # async fn rendering_control_example() -> Result<(), rupnp::Error> {
    /// # let some_url = unimplemented!();
    /// use rupnp::ssdp::URN;
    /// use rupnp::Device;
    ///
    /// let urn = URN::service("schemas-upnp-org", "RenderingControl", 1);
    ///
    /// let device = Device::from_url( some_url ).await?;
    /// let service = device.find_service(&urn)
    ///     .expect("service exists");
    ///
    /// let args = "<InstanceID>0</InstanceID><Channel>Master</Channel>";
    /// let response = service.action(device.url(), "GetVolume", args).await?;
    ///
    /// let volume = response
    ///     .get("CurrentVolume")
    ///     .expect("exists");
    ///
    /// println!("Volume: {}", volume);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn action(
        &self,
        url: &Uri,
        action: &str,
        payload: &str,
    ) -> Result<HashMap<String, String>> {
        self.action_with_timeout(url, action, payload, crate::DEFAULT_TIMEOUT)
            .await
    }

    /// Execute some UPnP Action on this service, giving up after `timeout`.
    ///
    /// This behaves like [`action`](Service::action), but the request is aborted when the
    /// device does not respond within `timeout`.
    pub async fn action_with_timeout(
        &self,
        url: &Uri,
        action: &str,
        payload: &str,
        timeout: Duration,
    ) -> Result<HashMap<String, String>> {
        let body = format!(
            r#"
            <s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"
                s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
                <s:Body>
                    <u:{action} xmlns:u="{service}">
                        {payload}
                    </u:{action}>
                </s:Body>
            </s:Envelope>"#,
            service = self.service_type,
            action = action,
            payload = payload
        );

        let soap_action = format!("\"{}#{}\"", self.service_type, action);

        let request = Request::post(self.control_url(url))
            .header("CONTENT-TYPE", "text/xml; charset=\"utf-8\"")
            .header("SOAPAction", soap_action)
            .body(body)
            .expect("infallible");

        let doc = utils::with_timeout(timeout, async {
            let response = Client::builder(TokioExecutor::new())
                .build_http()
                .request(request)
                .await?
                .err_if_not_200()?;

            response.into_body().bytes().await
        })
        .await?;

        let doc = std::str::from_utf8(&doc)?;
        debug!("Action server response: {}", doc);

        let document = Document::parse(doc)?;
        let response = utils::find_root(&document, "Body", "UPnP Response")?
            .first_element_child()
            .ok_or_else(|| {
                Error::XmlMissingElement("Body".to_string(), format!("{action}Response"))
            })?;

        if response.tag_name().name().eq_ignore_ascii_case("Fault") {
            return Err(UPnPError::from_fault_node(response)?.into());
        }

        let values: HashMap<_, _> = response
            .children()
            .filter(Node::is_element)
            .filter_map(|node| -> Option<(String, String)> {
                node.text()
                    .map(|text| (node.tag_name().name().to_string(), text.to_string()))
            })
            .collect();

        Ok(values)
    }

    #[cfg(feature = "subscribe")]
    async fn make_subscribe_request(
        &self,
        url: &Uri,
        callback: &str,
        timeout_secs: u32,
    ) -> Result<String> {
        use bytes::Bytes;
        use http_body_util::Empty;

        let event_sub_url = self
            .event_sub_url(url)
            .ok_or(Error::EventSubscriptionNotSupported)?;

        let req = Request::builder()
            .uri(event_sub_url)
            .method("SUBSCRIBE")
            .header("CALLBACK", format!("<{callback}>"))
            .header("NT", "upnp:event")
            .header("TIMEOUT", format!("Second-{timeout_secs}"))
            .body(Empty::<Bytes>::new())
            .expect("infallible");

        let response = utils::with_timeout(crate::DEFAULT_TIMEOUT, async {
            let response = Client::builder(TokioExecutor::new())
                .build_http()
                .request(req)
                .await?
                .err_if_not_200()?;

            Ok::<_, Error>(response)
        })
        .await?;

        let sid = response
            .headers()
            .get("sid")
            .ok_or_else(|| Error::ParseError("missing http header `SID`"))?
            .to_str()
            .map_err(|_| Error::ParseError("SID header contained non-visible ASCII bytes"))?
            .to_string();

        Ok(sid)
    }

    /// Subscribe for state variable changes.
    ///
    /// It returns the SID which can be used to unsubscribe to the service and a stream of
    /// responses.
    ///
    /// Each response is a [HashMap](std::collections::HashMap) of the state variables.
    ///
    /// # Example usage:
    /// ```rust,no_run
    /// # use futures::prelude::*;
    /// # async fn subscribe_example() -> Result<(), rupnp::Error> {
    /// # let device: rupnp::Device = unimplemented!();
    /// # let service: rupnp::Service = unimplemented!();
    /// let (_sid, stream) = service.subscribe(device.url(), 300).await?;
    ///
    /// while let Some(state_vars) = stream.try_next().await? {
    ///     for (key, value) in state_vars {
    ///         println!("{} => {}", key, value);
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "subscribe")]
    pub async fn subscribe(
        &self,
        url: &Uri,
        timeout_secs: u32,
    ) -> Result<(String, impl Stream<Item = Result<HashMap<String, String>>>)> {
        let callback_ip = utils::pick_local_addr(utils::peer_ip(url))?;
        self.subscribe_with_callback(url, callback_ip, timeout_secs)
            .await
    }

    /// Subscribe for state variable changes, using an explicit callback address.
    ///
    /// The device calls back to `http://<callback_ip>:<port>` to deliver the events, so the
    /// address has to be reachable from the device. Use this instead of
    /// [`subscribe`](Service::subscribe) if the automatic selection picks the wrong
    /// interface, for example on a machine with a VPN or a virtual machine adapter.
    ///
    /// # Example usage:
    /// ```rust,no_run
    /// # use futures::prelude::*;
    /// # use std::net::IpAddr;
    /// # async fn subscribe_example() -> Result<(), rupnp::Error> {
    /// # let device: rupnp::Device = unimplemented!();
    /// # let service: rupnp::Service = unimplemented!();
    /// let callback_ip: IpAddr = "192.168.1.10".parse().unwrap();
    /// let (_sid, stream) = service
    ///     .subscribe_with_callback(device.url(), callback_ip, 300)
    ///     .await?;
    ///
    /// while let Some(state_vars) = stream.try_next().await? {
    ///     for (key, value) in state_vars {
    ///         println!("{} => {}", key, value);
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "subscribe")]
    pub async fn subscribe_with_callback(
        &self,
        url: &Uri,
        callback_ip: IpAddr,
        timeout_secs: u32,
    ) -> Result<(String, impl Stream<Item = Result<HashMap<String, String>>>)> {
        let listener = TcpListener::bind(SocketAddr::new(callback_ip, 0)).await?;

        let callback = format_callback_addr(&listener.local_addr()?);

        let sid = self
            .make_subscribe_request(url, &callback, timeout_secs)
            .await?;

        let stream = Gen::new(move |co: Co<Result<_>>| subscribe_stream(listener, co));

        Ok((sid, stream))
    }

    /// Renew a subscription made with the [subscribe](struct.Service.html#method.subscribe) method.
    ///
    /// When the sid is invalid, the control point will respond with a `412 Preconditition failed`.
    pub async fn renew_subscription(&self, url: &Uri, sid: &str, timeout_secs: u32) -> Result<()> {
        let event_sub_url = self
            .event_sub_url(url)
            .ok_or(Error::EventSubscriptionNotSupported)?;

        let req = Request::builder()
            .uri(event_sub_url)
            .method("SUBSCRIBE")
            .header("SID", sid)
            .header("TIMEOUT", format!("Second-{timeout_secs}"))
            .body(Empty::<Bytes>::new())
            .expect("infallible");

        utils::with_timeout(crate::DEFAULT_TIMEOUT, async {
            Client::builder(TokioExecutor::new())
                .build_http()
                .request(req)
                .await?
                .err_if_not_200()?;

            Ok::<_, Error>(())
        })
        .await
    }

    /// Unsubscribe from further event notifications.
    ///
    /// The SID is usually obtained from the [subscribe](struct.Service.html#method.subscribe) method.
    ///
    /// When the sid is invalid, the control point will respond with a `412 Preconditition failed`.
    pub async fn unsubscribe(&self, url: &Uri, sid: &str) -> Result<()> {
        let event_sub_url = self
            .event_sub_url(url)
            .ok_or(Error::EventSubscriptionNotSupported)?;

        let req = Request::builder()
            .uri(event_sub_url)
            .method("UNSUBSCRIBE")
            .header("SID", sid)
            .body(Empty::<Bytes>::new())
            .expect("infallible");

        utils::with_timeout(crate::DEFAULT_TIMEOUT, async {
            Client::builder(TokioExecutor::new())
                .build_http()
                .request(req)
                .await?
                .err_if_not_200()?;

            Ok::<_, Error>(())
        })
        .await
    }
}

#[cfg(feature = "subscribe")]
macro_rules! yield_try {
    ( $co:expr => $expr:expr ) => {
        match $expr {
            Ok(val) => val,
            Err(e) => {
                $co.yield_(Err(e.into())).await;
                continue;
            }
        }
    };
}

#[cfg(feature = "subscribe")]
fn propertyset_to_map(input: &str) -> Result<HashMap<String, String>, roxmltree::Error> {
    let doc = Document::parse(input)?;
    let hashmap: HashMap<String, String> = doc
        .root_element() // <e:propertyset />
        .children() // <e:property />
        .filter_map(|child| child.first_element_child()) // actual tag
        .filter_map(|node| {
            node.text()
                .map(|text| (node.tag_name().name().to_string(), text.to_string()))
        })
        .collect();

    Ok(hashmap)
}

#[cfg(feature = "subscribe")]
async fn subscribe_stream(listener: TcpListener, co: Co<Result<HashMap<String, String>>>) {
    loop {
        let (mut stream, _) = yield_try!(co => listener.accept().await);
        let mut lines = BufReader::new(&mut stream).lines();

        let mut input = String::new();
        let mut is_xml = false;

        // sometimes the xml is on one line, sometimes on multiple ones.
        // we dont care about the http stuff before the "<e:propertyset>"
        while let Ok(Some(line)) = lines.next_line().await {
            if is_xml || line.starts_with("<e:propertyset") {
                input.push_str(&line);
                is_xml = true;
            }

            if line.ends_with("</e:propertyset>") {
                break;
            };
        }

        let response = "HTTP/1.1 200 OK\r\n\r\n";
        let _ = stream.write_all(response.as_bytes()).await;

        let hashmap = yield_try!(co => propertyset_to_map(&input));

        co.yield_(Ok(hashmap)).await;
    }
}

fn replace_url_path(url: &Uri, path: &PathAndQuery) -> Uri {
    let mut parts = url.clone().into_parts();
    parts.path_and_query = Some(path.clone());
    Uri::from_parts(parts).expect("infallible")
}

/// Formats a socket address as the callback url the device has to call.
#[cfg(feature = "subscribe")]
fn format_callback_addr(addr: &SocketAddr) -> String {
    match addr {
        SocketAddr::V4(addr) => format!("http://{}:{}", addr.ip(), addr.port()),
        SocketAddr::V6(addr) => {
            // The scope id has to be percent encoded in a url.
            let scope = if addr.scope_id() != 0 {
                format!("%25{}", addr.scope_id())
            } else {
                String::new()
            };
            format!("http://[{}{}]:{}", addr.ip(), scope, addr.port())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roxmltree::Document;

    const DEVICE_URL: &str = "http://192.168.1.5:1400/xml/device_description.xml";

    #[cfg(feature = "subscribe")]
    #[test]
    fn callback_addr_is_formatted_for_ipv4_and_ipv6() {
        assert_eq!(
            format_callback_addr(&"192.168.1.10:1234".parse().unwrap()),
            "http://192.168.1.10:1234"
        );
        assert_eq!(
            format_callback_addr(&"[fe80::1]:1234".parse().unwrap()),
            "http://[fe80::1]:1234"
        );
    }

    fn service_from_xml(xml: &str) -> Result<Service, Box<dyn std::error::Error>> {
        let doc = Document::parse(xml)?;
        Ok(Service::from_xml(doc.root_element())?)
    }

    #[test]
    fn service_without_event_sub_url_is_parsed() -> Result<(), Box<dyn std::error::Error>> {
        let service = service_from_xml(
            r#"
            <service>
                <serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>
                <serviceId>urn:upnp-org:serviceId:RenderingControl</serviceId>
                <SCPDURL>/MediaRenderer/RenderingControl/Description.xml</SCPDURL>
                <controlURL>/MediaRenderer/RenderingControl/control</controlURL>
            </service>
            "#,
        )?;

        assert!(!service.supports_events());
        assert_eq!(service.event_sub_url(&Uri::from_static(DEVICE_URL)), None);

        Ok(())
    }

    #[test]
    fn service_with_event_sub_url_is_parsed() -> Result<(), Box<dyn std::error::Error>> {
        let service = service_from_xml(
            r#"
            <service>
                <serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>
                <serviceId>urn:upnp-org:serviceId:RenderingControl</serviceId>
                <SCPDURL>/MediaRenderer/RenderingControl/Description.xml</SCPDURL>
                <controlURL>/MediaRenderer/RenderingControl/control</controlURL>
                <eventSubURL>/MediaRenderer/RenderingControl/Event</eventSubURL>
            </service>
            "#,
        )?;

        assert!(service.supports_events());
        assert_eq!(
            service
                .event_sub_url(&Uri::from_static(DEVICE_URL))
                .map(|url| url.to_string()),
            Some("http://192.168.1.5:1400/MediaRenderer/RenderingControl/Event".to_string())
        );

        Ok(())
    }

    #[test]
    fn service_with_relative_endpoints_is_parsed() -> Result<(), Box<dyn std::error::Error>> {
        let service = service_from_xml(
            r#"
            <service>
                <serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>
                <serviceId>urn:upnp-org:serviceId:RenderingControl</serviceId>
                <SCPDURL>MediaRenderer/RenderingControl/Description.xml</SCPDURL>
                <controlURL>MediaRenderer/RenderingControl/control</controlURL>
                <eventSubURL>MediaRenderer/RenderingControl/Event</eventSubURL>
            </service>
            "#,
        )?;

        assert!(service.supports_events());
        assert_eq!(
            service
                .control_url(&Uri::from_static(DEVICE_URL))
                .to_string(),
            "http://192.168.1.5:1400/MediaRenderer/RenderingControl/control"
        );

        Ok(())
    }
}
