//! Extraction engine for scripted extensions.
//!
//! An extension describes *where* a value lives rather than *how* to fetch it:
//! a CSS selector for HTML documents, a JSON pointer for JSON ones, plus
//! optional post-processing (attribute pick, regex, prefix/suffix, value map,
//! date parsing). This module evaluates those descriptions.

use std::collections::BTreeMap;
use std::collections::HashMap;

use scraper::{ElementRef, Html, Selector};
use serde::Deserialize;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Declarative specs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Extract {
    /// Shorthand form: a bare selector (HTML) or pointer (JSON), text content.
    Path(String),
    Spec(Box<ExtractSpec>),
}

impl Extract {
    pub fn spec(&self) -> ExtractSpec {
        match self {
            Self::Path(path) => ExtractSpec {
                selector: Some(path.clone()),
                ..Default::default()
            },
            Self::Spec(spec) => (**spec).clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
// camelCase throughout, matching the rest of the manifest format.
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ExtractSpec {
    /// CSS selector, or JSON pointer when the document is JSON.
    /// Absent means "use the current node".
    #[serde(default)]
    pub selector: Option<String>,
    /// `text` (default), `html`, or an attribute name such as `src`.
    #[serde(default)]
    pub attr: Option<String>,
    /// Collect every match instead of only the first.
    #[serde(default)]
    pub all: bool,
    /// JSON only: keep the array element whose `find` pointer equals `equals`.
    /// Lets an extension pick, say, the `cover_art` entry out of a list of
    /// heterogeneous relationships.
    #[serde(default)]
    pub find: Option<String>,
    #[serde(default)]
    pub equals: Option<String>,
    /// JSON only: pointer applied inside the element selected by `find`.
    #[serde(default)]
    pub then: Option<String>,
    #[serde(default)]
    pub regex: Option<RegexSpec>,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub suffix: Option<String>,
    /// `chrono` format string used to turn the value into a timestamp.
    #[serde(default)]
    pub date_format: Option<String>,
    /// Literal remapping applied after everything else, e.g. status labels.
    #[serde(default)]
    pub map: BTreeMap<String, String>,
    #[serde(default)]
    pub default: Option<String>,
    /// Builds a value out of several extractions: `{0}`, `{1}`, … are replaced
    /// by the results of `parts`, evaluated against the same node. Needed when
    /// a url is assembled from more than one field.
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub parts: Vec<ExtractSpec>,
    /// Resolve this part against the document root instead of the current item.
    /// Page urls typically mix a per-document base with a per-item file name.
    #[serde(default)]
    pub from_root: bool,
    /// Tried in order when this spec yields nothing. Localised APIs often key
    /// a field by language, so the preferred one may simply be absent.
    #[serde(default)]
    pub alternatives: Vec<ExtractSpec>,
    /// JSON only: when the target is an object, take its first value. Pairs
    /// with `alternatives` to mean "whatever translation exists".
    #[serde(default)]
    pub first_value: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegexSpec {
    pub pattern: String,
    /// Replacement template; `$1` style groups are supported.
    /// When absent, the first capture group (or whole match) is taken.
    #[serde(default)]
    pub replace: Option<String>,
}

// ---------------------------------------------------------------------------
// Documents and nodes
// ---------------------------------------------------------------------------

pub enum Document {
    Html(Html),
    Json(Value),
}

impl Document {
    pub fn parse(body: &str, is_json: bool) -> anyhow::Result<Self> {
        if is_json {
            Ok(Self::Json(serde_json::from_str(body)?))
        } else {
            Ok(Self::Html(Html::parse_document(body)))
        }
    }

    pub fn root(&self) -> Node<'_> {
        match self {
            Self::Html(html) => Node::Html(html.root_element()),
            Self::Json(value) => Node::Json(value),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Node<'a> {
    Html(ElementRef<'a>),
    Json(&'a Value),
}

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

/// Caches compiled selectors for the lifetime of one request, so that a list of
/// fifty items does not recompile the same selector fifty times.
#[derive(Default)]
pub struct Evaluator {
    selectors: HashMap<String, Option<Selector>>,
}

impl Evaluator {
    pub fn new() -> Self {
        Self::default()
    }

    fn selector(&mut self, css: &str) -> Option<&Selector> {
        self.selectors
            .entry(css.to_string())
            .or_insert_with(|| match Selector::parse(css) {
                Ok(selector) => Some(selector),
                Err(err) => {
                    log::warn!("invalid CSS selector {css:?}: {err}");
                    None
                }
            })
            .as_ref()
    }

    /// Selects the repeating elements a list request iterates over.
    pub fn select_items<'a>(&mut self, node: Node<'a>, path: &str) -> Vec<Node<'a>> {
        match node {
            Node::Html(element) => match self.selector(path) {
                Some(selector) => element.select(selector).map(Node::Html).collect(),
                None => Vec::new(),
            },
            Node::Json(value) => match value.pointer(path) {
                Some(Value::Array(items)) => items.iter().map(Node::Json).collect(),
                Some(other) => vec![Node::Json(other)],
                None => Vec::new(),
            },
        }
    }

    /// Resolves a single value, or `None` when nothing matches.
    pub fn extract(&mut self, node: Node<'_>, extract: &Extract) -> Option<String> {
        self.extract_in(node, node, extract)
    }

    /// Like [`Self::extract`], but parts marked `fromRoot` resolve against `root`.
    pub fn extract_in(
        &mut self,
        node: Node<'_>,
        root: Node<'_>,
        extract: &Extract,
    ) -> Option<String> {
        let spec = extract.spec();
        if let Some(template) = spec.format.clone() {
            return self.compose(node, root, &template, &spec);
        }

        let source = if spec.from_root { root } else { node };
        let raw = self.raw_values(source, &spec, false).into_iter().next();
        // `default` must not pre-empt the alternatives, so it is applied last.
        let bare = ExtractSpec {
            default: None,
            ..spec.clone()
        };
        if let Some(value) = self.finish(raw, &bare) {
            return Some(value);
        }

        for alternative in &spec.alternatives {
            let source = if alternative.from_root { root } else { node };
            let raw = self
                .raw_values(source, alternative, false)
                .into_iter()
                .next();
            if let Some(value) = self.finish(raw, alternative) {
                return Some(value);
            }
        }
        spec.default.clone()
    }

    /// Fills `{0}`, `{1}`, … in `template` from `spec.parts`.
    fn compose(
        &mut self,
        node: Node<'_>,
        root: Node<'_>,
        template: &str,
        spec: &ExtractSpec,
    ) -> Option<String> {
        let mut result = template.to_string();
        for (index, part) in spec.parts.iter().enumerate() {
            let source = if part.from_root { root } else { node };
            let raw = self.raw_values(source, part, false).into_iter().next();
            // A missing part invalidates the whole value: a half-built url is
            // worse than none at all.
            let value = self.finish(raw, part)?;
            result = result.replace(&format!("{{{index}}}"), &value);
        }
        Some(result)
    }

    /// Resolves every match, joined by the caller (used for genre lists).
    pub fn extract_all(&mut self, node: Node<'_>, extract: &Extract) -> Vec<String> {
        let spec = extract.spec();
        self.raw_values(node, &spec, true)
            .into_iter()
            .filter_map(|value| self.finish(Some(value), &spec))
            .collect()
    }

    pub fn extract_or_empty(
        &mut self,
        node: Node<'_>,
        extract: &Option<Extract>,
    ) -> Option<String> {
        extract.as_ref().and_then(|e| self.extract(node, e))
    }

    /// Resolves a timestamp in milliseconds, honouring `dateFormat`.
    pub fn extract_date(&mut self, node: Node<'_>, extract: &Option<Extract>) -> i64 {
        let Some(extract) = extract else { return 0 };
        let spec = extract.spec();
        let Some(value) = self.extract(node, extract) else {
            return 0;
        };
        parse_date(&value, spec.date_format.as_deref())
    }

    fn raw_values(&mut self, node: Node<'_>, spec: &ExtractSpec, all: bool) -> Vec<String> {
        let want_all = all || spec.all;
        let attr = spec.attr.as_deref().unwrap_or("text");

        match node {
            Node::Html(element) => {
                let targets: Vec<ElementRef<'_>> = match spec.selector.as_deref() {
                    None => vec![element],
                    Some(css) => match self.selector(css) {
                        Some(selector) => {
                            if want_all {
                                element.select(selector).collect()
                            } else {
                                element.select(selector).take(1).collect()
                            }
                        }
                        None => Vec::new(),
                    },
                };
                targets
                    .into_iter()
                    .filter_map(|target| html_value(target, attr))
                    .collect()
            }
            Node::Json(value) => {
                let target = match spec.selector.as_deref() {
                    None | Some("") => Some(value),
                    Some(pointer) => value.pointer(pointer),
                };

                // Narrow an array down to one element before reading a value.
                let target = match (&spec.find, target) {
                    (Some(field), Some(Value::Array(items))) => {
                        let wanted = spec.equals.as_deref();
                        items
                            .iter()
                            .find(|item| {
                                let actual = item.pointer(field).and_then(Value::as_str);
                                match wanted {
                                    Some(wanted) => actual == Some(wanted),
                                    None => actual.is_some(),
                                }
                            })
                            .and_then(|found| match spec.then.as_deref() {
                                Some(pointer) => found.pointer(pointer),
                                None => Some(found),
                            })
                    }
                    (_, other) => other,
                };

                match target {
                    // With `all`, `then` addresses a field inside each element,
                    // which is how tag lists are usually shaped.
                    Some(Value::Array(items)) if want_all => items
                        .iter()
                        .filter_map(|item| match spec.then.as_deref() {
                            Some(pointer) if spec.find.is_none() => {
                                item.pointer(pointer).and_then(json_value)
                            }
                            _ => json_value(item),
                        })
                        .collect(),
                    Some(Value::Array(items)) => items
                        .iter()
                        .next()
                        .and_then(json_value)
                        .into_iter()
                        .collect(),
                    // "Whatever value is there", for language-keyed objects.
                    Some(Value::Object(map)) if spec.first_value => {
                        map.values().find_map(json_value).into_iter().collect()
                    }
                    Some(other) => json_value(other).into_iter().collect(),
                    None => Vec::new(),
                }
            }
        }
    }

    fn finish(&self, raw: Option<String>, spec: &ExtractSpec) -> Option<String> {
        let mut value = match raw {
            Some(value) => value,
            None => return spec.default.clone(),
        };

        if let Some(regex) = &spec.regex {
            value = match apply_regex(&value, regex) {
                Some(applied) => applied,
                None => return spec.default.clone(),
            };
        }

        value = value.trim().to_string();

        if let Some(mapped) = spec.map.get(&value) {
            value = mapped.clone();
        } else if !spec.map.is_empty() {
            // Fall back to a case-insensitive lookup before giving up on the map.
            let lowered = value.to_lowercase();
            if let Some(mapped) = spec
                .map
                .iter()
                .find(|(key, _)| key.to_lowercase() == lowered)
                .map(|(_, v)| v.clone())
            {
                value = mapped;
            }
        }

        if let Some(prefix) = &spec.prefix {
            value = format!("{prefix}{value}");
        }
        if let Some(suffix) = &spec.suffix {
            value = format!("{value}{suffix}");
        }

        if value.is_empty() {
            return spec.default.clone();
        }
        Some(value)
    }
}

fn html_value(element: ElementRef<'_>, attr: &str) -> Option<String> {
    let value = match attr {
        "text" => element.text().collect::<String>(),
        "html" | "innerHtml" => element.inner_html(),
        "outerHtml" => element.html(),
        other => element.value().attr(other)?.to_string(),
    };
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn json_value(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => return None,
        other => other.to_string(),
    };
    (!text.is_empty()).then_some(text)
}

fn apply_regex(value: &str, spec: &RegexSpec) -> Option<String> {
    let regex = regex::Regex::new(&spec.pattern)
        .map_err(|err| log::warn!("invalid regex {:?}: {err}", spec.pattern))
        .ok()?;

    match &spec.replace {
        // With a template, rewrite the whole value.
        Some(template) => Some(regex.replace_all(value, template.as_str()).into_owned()),
        // Without one, pull out the first group (or the whole match).
        None => {
            let captures = regex.captures(value)?;
            captures
                .get(1)
                .or_else(|| captures.get(0))
                .map(|m| m.as_str().to_string())
        }
    }
}

/// Parses a date string into epoch milliseconds.
///
/// Supports an explicit `chrono` format, ISO-8601, bare epoch values, and the
/// "3 days ago" style relative labels that scanlation sites like to use.
pub fn parse_date(value: &str, format: Option<&str>) -> i64 {
    let value = value.trim();
    if value.is_empty() {
        return 0;
    }

    if let Some(format) = format {
        if let Ok(date) = chrono::NaiveDate::parse_from_str(value, format)
            && let Some(dt) = date.and_hms_opt(0, 0, 0)
        {
            return dt.and_utc().timestamp_millis();
        }
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(value, format) {
            return dt.and_utc().timestamp_millis();
        }
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return dt.timestamp_millis();
    }

    if let Ok(epoch) = value.parse::<i64>() {
        // Heuristic: ten digits is seconds, thirteen is milliseconds.
        return if value.len() <= 10 {
            epoch * 1000
        } else {
            epoch
        };
    }

    if let Some(millis) = parse_relative_date(value) {
        return millis;
    }

    for fallback in ["%Y-%m-%d", "%d/%m/%Y", "%m/%d/%Y", "%b %d, %Y", "%d %b %Y"] {
        if let Ok(date) = chrono::NaiveDate::parse_from_str(value, fallback)
            && let Some(dt) = date.and_hms_opt(0, 0, 0)
        {
            return dt.and_utc().timestamp_millis();
        }
    }

    0
}

fn parse_relative_date(value: &str) -> Option<i64> {
    let lowered = value.to_lowercase();
    if lowered.contains("now") || lowered.contains("just") {
        return Some(chrono::Utc::now().timestamp_millis());
    }

    let regex = regex::Regex::new(r"(\d+)\s*(second|minute|hour|day|week|month|year)").ok()?;
    let captures = regex.captures(&lowered)?;
    let amount: i64 = captures.get(1)?.as_str().parse().ok()?;
    let unit = captures.get(2)?.as_str();

    let seconds = match unit {
        "second" => 1,
        "minute" => 60,
        "hour" => 3_600,
        "day" => 86_400,
        "week" => 604_800,
        "month" => 2_592_000,
        "year" => 31_536_000,
        _ => return None,
    };
    Some(chrono::Utc::now().timestamp_millis() - amount * seconds * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html_doc(body: &str) -> Document {
        Document::parse(body, false).unwrap()
    }

    #[test]
    fn extracts_text_and_attributes_from_html() {
        let doc = html_doc(r#"<div class="c"><a href="/x">Title</a><img src="/i.png"></div>"#);
        let mut eval = Evaluator::new();
        let items = eval.select_items(doc.root(), "div.c");
        assert_eq!(items.len(), 1);

        let title = eval.extract(items[0], &Extract::Path("a".into()));
        assert_eq!(title.as_deref(), Some("Title"));

        let href = eval.extract(
            items[0],
            &Extract::Spec(Box::new(ExtractSpec {
                selector: Some("a".into()),
                attr: Some("href".into()),
                ..Default::default()
            })),
        );
        assert_eq!(href.as_deref(), Some("/x"));
    }

    #[test]
    fn extracts_from_json_pointers() {
        let doc = Document::parse(r#"{"data":[{"t":"A"},{"t":"B"}]}"#, true).unwrap();
        let mut eval = Evaluator::new();
        let items = eval.select_items(doc.root(), "/data");
        assert_eq!(items.len(), 2);
        assert_eq!(
            eval.extract(items[1], &Extract::Path("/t".into()))
                .as_deref(),
            Some("B")
        );
    }

    #[test]
    fn regex_and_map_post_processing() {
        let doc = html_doc("<span>Chapter 42 released</span>");
        let mut eval = Evaluator::new();
        let value = eval.extract(
            doc.root(),
            &Extract::Spec(Box::new(ExtractSpec {
                selector: Some("span".into()),
                regex: Some(RegexSpec {
                    pattern: r"Chapter (\d+)".into(),
                    replace: None,
                }),
                ..Default::default()
            })),
        );
        assert_eq!(value.as_deref(), Some("42"));
    }

    #[test]
    fn dates_parse_from_several_shapes() {
        assert!(parse_date("2020-01-02", None) > 0);
        assert!(parse_date("2020-01-02T03:04:05+00:00", None) > 0);
        assert!(parse_date("1577934245", None) > 0);
        assert!(parse_date("3 days ago", None) > 0);
        assert_eq!(parse_date("", None), 0);
    }
}
