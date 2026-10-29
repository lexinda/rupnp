//! Helpers for building XML payloads.
//!
//! UPnP actions take their arguments as an XML fragment which is embedded into the SOAP
//! body. The values of those arguments have to be escaped, otherwise a media url with an
//! ampersand in its query string or a title with a `<` in it produces an invalid document
//! and the device rejects the request.

/// Escapes a string so that it can be used as XML text content.
///
/// In addition to the characters that are illegal in XML this also escapes carriage
/// returns, because an XML parser would turn them into line feeds.
pub fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());

    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\r' => escaped.push_str("&#xD;"),
            _ => escaped.push(ch),
        }
    }

    escaped
}

/// Escapes a string so that it can be used as an XML attribute value.
///
/// In addition to the characters that are illegal in XML this also escapes the whitespace
/// characters that an XML parser would otherwise normalize.
pub fn escape_attr(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());

    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '\t' => escaped.push_str("&#x9;"),
            '\n' => escaped.push_str("&#xA;"),
            '\r' => escaped.push_str("&#xD;"),
            _ => escaped.push(ch),
        }
    }

    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_escapes_the_special_characters() {
        assert_eq!(escape_text("a & b"), "a &amp; b");
        assert_eq!(escape_text("<tag>"), "&lt;tag&gt;");
        assert_eq!(escape_text("\"quoted\""), "\"quoted\"");
        assert_eq!(escape_text("it's"), "it's");
    }

    #[test]
    fn text_escapes_the_ampersand_first() {
        // The ampersand of an already escaped entity must not be escaped twice.
        assert_eq!(escape_text("&amp;"), "&amp;amp;");
    }

    #[test]
    fn text_escapes_carriage_returns() {
        assert_eq!(escape_text("a\r\nb"), "a&#xD;\nb");
    }

    #[test]
    fn attr_escapes_the_special_characters() {
        assert_eq!(escape_attr("a & b"), "a &amp; b");
        assert_eq!(escape_attr("<tag>"), "&lt;tag&gt;");
        assert_eq!(escape_attr("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(escape_attr("it's"), "it&apos;s");
    }

    #[test]
    fn attr_escapes_the_whitespace_characters() {
        assert_eq!(escape_attr("a\tb\nc\rd"), "a&#x9;b&#xA;c&#xD;d");
    }

    #[test]
    fn escaped_text_is_valid_xml() {
        let text = "Fish & Chips <b>bold</b> \"quoted\" 'single'";
        let xml = format!("<value>{}</value>", escape_text(text));

        let doc = roxmltree::Document::parse(&xml).unwrap();
        assert_eq!(doc.root_element().text(), Some(text));
    }

    #[test]
    fn escaped_attr_is_valid_xml() {
        let text = "Fish & Chips <b>bold</b> \"quoted\" 'single'";
        let xml = format!("<value attr=\"{}\"/>", escape_attr(text));

        let doc = roxmltree::Document::parse(&xml).unwrap();
        assert_eq!(doc.root_element().attribute("attr"), Some(text));
    }
}
