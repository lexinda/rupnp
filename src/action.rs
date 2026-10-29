//! The response of a UPnP action.

use crate::{Error, Result};
use std::collections::HashMap;

/// The response of a UPnP action.
///
/// The arguments are kept in the order the device returned them, which is the order the
/// service description defines them in. Unlike a [`HashMap`] this makes it possible to
/// read the arguments of an action by position.
///
/// # Example usage:
/// ```rust,no_run
/// # use futures::prelude::*;
/// # use rupnp::ssdp::URN;
/// # async fn position_example() -> Result<(), rupnp::Error> {
/// # let device: rupnp::Device = unimplemented!();
/// # let service: rupnp::Service = unimplemented!();
/// let response = service
///     .action_ordered(device.url(), "GetPositionInfo", "<InstanceID>0</InstanceID>")
///     .await?;
///
/// let track: u32 = response.get_u32("Track")?;
/// let duration = response.get("TrackDuration").unwrap_or_default();
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionResponse {
    entries: Vec<(String, String)>,
}

impl ActionResponse {
    /// Creates an empty response.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the value of the argument with the given name.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Returns the value of the argument with the given name, parsed as an `u32`.
    pub fn get_u32(&self, name: &str) -> Result<u32> {
        self.get_parsed(name)
    }

    /// Returns the value of the argument with the given name, parsed as an `i32`.
    pub fn get_i32(&self, name: &str) -> Result<i32> {
        self.get_parsed(name)
    }

    /// Returns the value of the argument with the given name, parsed as an `f32`.
    pub fn get_f32(&self, name: &str) -> Result<f32> {
        self.get_parsed(name)
    }

    /// Returns the value of the argument with the given name, parsed as a `bool`.
    ///
    /// UPnP booleans are `0` or `1`, but `true` and `false` are accepted as well.
    pub fn get_bool(&self, name: &str) -> Result<bool> {
        let value = self
            .get(name)
            .ok_or_else(|| Error::MissingArgument(name.to_string()))?;

        match value {
            "1" | "true" | "TRUE" | "True" => Ok(true),
            "0" | "false" | "FALSE" | "False" => Ok(false),
            _ => Err(Error::invalid_response(ParseBoolError(value.to_string()))),
        }
    }

    /// Returns the argument with the given name, parsed into the given type.
    fn get_parsed<T>(&self, name: &str) -> Result<T>
    where
        T: std::str::FromStr,
        T::Err: std::error::Error + Send + Sync + 'static,
    {
        let value = self
            .get(name)
            .ok_or_else(|| Error::MissingArgument(name.to_string()))?;

        value.parse().map_err(Error::invalid_response)
    }

    /// Returns the arguments in the order the device returned them.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    /// Returns the number of arguments.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the response contains no arguments.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Converts the response into a map, losing the order of the arguments.
    pub fn into_map(self) -> HashMap<String, String> {
        self.entries.into_iter().collect()
    }

    pub(crate) fn from_entries(entries: Vec<(String, String)>) -> Self {
        Self { entries }
    }
}

impl From<ActionResponse> for HashMap<String, String> {
    fn from(response: ActionResponse) -> Self {
        response.into_map()
    }
}

impl IntoIterator for ActionResponse {
    type Item = (String, String);
    type IntoIter = std::vec::IntoIter<(String, String)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<'a> IntoIterator for &'a ActionResponse {
    type Item = (&'a str, &'a str);
    type IntoIter = Box<dyn Iterator<Item = (&'a str, &'a str)> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

/// The error returned when a boolean argument cannot be parsed.
#[derive(Debug)]
pub struct ParseBoolError(String);

impl std::fmt::Display for ParseBoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid boolean: `{}`", self.0)
    }
}

impl std::error::Error for ParseBoolError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> ActionResponse {
        ActionResponse::from_entries(vec![
            ("Track".to_string(), "1".to_string()),
            ("TrackDuration".to_string(), "0:10:34.000".to_string()),
            ("RelCount".to_string(), "-5".to_string()),
            ("AbsCount".to_string(), "42".to_string()),
            ("Enabled".to_string(), "1".to_string()),
        ])
    }

    #[test]
    fn keeps_the_order_of_the_arguments() {
        let response = response();
        let names = response.iter().map(|(name, _)| name).collect::<Vec<_>>();

        assert_eq!(
            names,
            ["Track", "TrackDuration", "RelCount", "AbsCount", "Enabled"]
        );
    }

    #[test]
    fn reads_the_arguments_by_name() {
        let response = response();

        assert_eq!(response.get("Track"), Some("1"));
        assert_eq!(response.get("TrackDuration"), Some("0:10:34.000"));
        assert_eq!(response.get("Missing"), None);
    }

    #[test]
    fn parses_the_typed_arguments() {
        let response = response();

        assert_eq!(response.get_u32("Track").unwrap(), 1);
        assert_eq!(response.get_u32("AbsCount").unwrap(), 42);
        assert_eq!(response.get_i32("RelCount").unwrap(), -5);
        assert_eq!(response.get_f32("Track").unwrap(), 1.0);
        assert!(response.get_bool("Enabled").unwrap());
    }

    #[test]
    fn reports_missing_arguments() {
        let response = response();

        assert!(matches!(
            response.get_u32("Missing"),
            Err(Error::MissingArgument(_))
        ));
        assert!(matches!(
            response.get_bool("Missing"),
            Err(Error::MissingArgument(_))
        ));
    }

    #[test]
    fn reports_invalid_values() {
        let response =
            ActionResponse::from_entries(vec![("Enabled".to_string(), "maybe".to_string())]);

        assert!(response.get_bool("Enabled").is_err());
    }

    #[test]
    fn converts_into_a_map() {
        let map = response().into_map();

        assert_eq!(map.get("Track").map(String::as_str), Some("1"));
        assert_eq!(map.len(), 5);
    }
}
