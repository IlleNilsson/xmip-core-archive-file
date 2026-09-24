//! The sidecar beside an archived file — `<item>.meta` — as TOML written and
//! read by hand: an `[item]` table naming what was archived and when, and a
//! `[metadata]` table of the item's pairs. An operator reads it in any editor,
//! and the crate carries no TOML dependency for two tables of strings. Every
//! key and value is quoted and read back through `codec::toml`, the one TOML
//! basic string the estate writes.

use codec::toml::{quote, unquote, unquote_prefix};

/// What the sidecar records.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Meta {
    pub data_type: String,
    pub identifier: String,
    pub archived_at: String,
    pub metadata: Vec<(String, String)>,
}

impl Meta {
    /// The sidecar text. Every key and every value is a TOML basic string, so
    /// an identifier with a space or a quote in it needs no special case. A
    /// pair whose key repeats is written twice, in order; a strict TOML reader
    /// objects to that, [`Meta::parse`] keeps the order.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut text = String::from("[item]\n");
        push_pair(&mut text, "data_type", &self.data_type);
        push_pair(&mut text, "identifier", &self.identifier);
        push_pair(&mut text, "archived_at", &self.archived_at);
        text.push_str("\n[metadata]\n");
        for (key, value) in &self.metadata {
            push_pair(&mut text, key, value);
        }
        text
    }

    /// A sidecar read back.
    ///
    /// # Errors
    ///
    /// A line that is not a table header or a `key = "value"` pair, a table
    /// other than the two, a value that is not a basic string, or an `[item]`
    /// table missing one of its three keys.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut table = "";
        let mut data_type = None;
        let mut identifier = None;
        let mut archived_at = None;
        let mut metadata = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |reason: &str| format!("line {}: {reason}", index + 1);
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                table = match name.trim() {
                    "item" => "item",
                    "metadata" => "metadata",
                    other => return Err(at(&format!("unknown table [{other}]"))),
                };
                continue;
            }
            let (key, value) = pair(line).map_err(|reason| at(&reason))?;
            match (table, key.as_str()) {
                ("item", "data_type") => data_type = Some(value),
                ("item", "identifier") => identifier = Some(value),
                ("item", "archived_at") => archived_at = Some(value),
                ("item", other) => return Err(at(&format!("unknown item key {other}"))),
                ("metadata", _) => metadata.push((key, value)),
                _ => return Err(at("a pair before any table")),
            }
        }
        Ok(Self {
            data_type: data_type.ok_or("[item] has no data_type")?,
            identifier: identifier.ok_or("[item] has no identifier")?,
            archived_at: archived_at.ok_or("[item] has no archived_at")?,
            metadata,
        })
    }
}

fn push_pair(text: &mut String, key: &str, value: &str) {
    text.push_str(&quote(key));
    text.push_str(" = ");
    text.push_str(&quote(value));
    text.push('\n');
}

/// A `key = "value"` line: the key quoted, which is what this crate writes
/// and may hold `=`, or bare.
fn pair(line: &str) -> Result<(String, String), String> {
    let (key, rest) = if line.starts_with('"') {
        unquote_prefix(line).map_err(|error| error.to_string())?
    } else {
        let end = line
            .find(|c: char| c == '=' || c.is_whitespace())
            .unwrap_or(line.len());
        let key = &line[..end];
        let bare = !key.is_empty()
            && key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !bare {
            return Err(format!("{key} is not a key"));
        }
        (key.to_string(), &line[end..])
    };
    let value = rest
        .trim_start()
        .strip_prefix('=')
        .ok_or("not a key = value pair")?;
    let value = unquote(value.trim()).map_err(|error| error.to_string())?;
    Ok((key, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Meta {
        Meta {
            data_type: "json".to_string(),
            identifier: "it's \"odd\" \\ id\u{1f}x".to_string(),
            archived_at: "2026-09-09T00:00:00Z".to_string(),
            metadata: vec![
                ("source".to_string(), "playground".to_string()),
                (
                    "note with space".to_string(),
                    "two\nlines\ttabbed".to_string(),
                ),
            ],
        }
    }

    #[test]
    fn the_sidecar_round_trips_through_toml() {
        let text = meta().to_toml();
        assert_eq!(Meta::parse(&text).expect("parse"), meta());
    }

    #[test]
    fn the_sidecar_has_the_two_tables_and_the_escapes() {
        let text = meta().to_toml();
        assert!(
            text.starts_with("[item]\n\"data_type\" = \"json\"\n"),
            "{text}"
        );
        assert!(
            text.contains("\n[metadata]\n\"source\" = \"playground\"\n"),
            "{text}"
        );
        assert!(text.contains(r#""two\nlines\ttabbed""#), "{text}");
        let escaped = format!("{}u001Fx", r#"\"odd\" \\ id\"#);
        assert!(text.contains(&escaped), "{text}");
    }

    #[test]
    fn a_bare_key_and_a_comment_are_read() {
        let text = "# hand-written\n[item]\ndata_type = \"csv\"\nidentifier = \"a\"\n\
                    archived_at = \"t\"\n[metadata]\nk-1 = \"v\"\n";
        let parsed = Meta::parse(text).expect("parse");
        assert_eq!(parsed.data_type, "csv");
        assert_eq!(parsed.metadata, vec![("k-1".to_string(), "v".to_string())]);
    }

    #[test]
    fn a_key_holding_an_equals_sign_and_every_control_character_round_trip() {
        let odd = Meta {
            metadata: vec![
                ("a = b".to_string(), "c = \"d\"".to_string()),
                (
                    "\u{0}\r\u{7f}".to_string(),
                    "Zoë\u{a0}名前\u{1b}".to_string(),
                ),
            ],
            ..meta()
        };
        let text = odd.to_toml();
        assert!(text.contains(r#""\u0000\r\u007F""#), "{text}");
        assert_eq!(Meta::parse(&text).expect("parse"), odd);
    }

    #[test]
    fn a_missing_item_key_is_an_error() {
        let refused = Meta::parse("[item]\n\"data_type\" = \"csv\"\n").expect_err("incomplete");
        assert!(refused.contains("identifier"), "{refused}");
    }

    #[test]
    fn a_value_that_is_not_a_string_is_an_error() {
        let refused = Meta::parse("[item]\n\"data_type\" = 3\n").expect_err("not a string");
        assert!(refused.contains("line 2"), "{refused}");
    }
}
