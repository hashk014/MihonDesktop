//! Scripted extensions.
//!
//! Mihon's extensions are Android APKs containing compiled Kotlin, which a Rust
//! desktop binary cannot load. The equivalent here is declarative: an extension
//! is a JSON document describing endpoints and where each value sits in the
//! response. [`ScriptedSource`] interprets one at runtime, so adding a site
//! means dropping a file into `extensions/` — no recompilation.

pub mod eval;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde::Deserialize;

use self::eval::{Document, Evaluator, Extract, Node, RegexSpec};
use super::{
    Filter, FilterKind, FilterList, MangasPage, Page, SChapter, SManga, Source, for_each_filter,
    generate_id,
};
use crate::model::{MangaStatus, TriState, UpdateStrategy};
use crate::net::{HttpClient, absolute_url};

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionManifest {
    /// Stable identifier, also used as the file name.
    pub id: String,
    pub name: String,
    pub lang: String,
    pub base_url: String,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub nsfw: bool,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub image_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub rate_limit: Option<RateLimitSpec>,
    /// Results per page, used to expand the `{offset}` url placeholder.
    #[serde(default = "default_page_size")]
    pub page_size: u32,

    #[serde(default)]
    pub popular: Option<ListRequest>,
    #[serde(default)]
    pub latest: Option<ListRequest>,
    #[serde(default)]
    pub search: Option<ListRequest>,
    pub details: DetailsRequest,
    pub chapters: ChapterRequest,
    pub pages: PageRequest,
    #[serde(default)]
    pub filters: Vec<FilterSpec>,
}

fn default_version() -> u32 {
    1
}

fn default_page_size() -> u32 {
    20
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitSpec {
    pub permits: usize,
    pub period_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListRequest {
    /// Supports `{base}`, `{page}`, `{query}` and `{filters}` placeholders.
    pub url: String,
    #[serde(default)]
    pub json: bool,
    pub list: MangaListSpec,
    /// Resolving to a non-empty, non-false value means another page exists.
    #[serde(default)]
    pub next_page: Option<Extract>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaListSpec {
    /// Selector (or JSON pointer) for the repeating element.
    pub item: String,
    pub title: Extract,
    pub url: Extract,
    #[serde(default)]
    pub thumbnail: Option<Extract>,
    /// Items where this resolves to a value are dropped.
    #[serde(default)]
    pub skip_if: Option<Extract>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailsRequest {
    pub url: String,
    #[serde(default)]
    pub json: bool,
    /// Narrows the document down before the fields are read.
    #[serde(default)]
    pub root: Option<String>,
    pub fields: DetailsFields,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailsFields {
    #[serde(default)]
    pub title: Option<Extract>,
    #[serde(default)]
    pub author: Option<Extract>,
    #[serde(default)]
    pub artist: Option<Extract>,
    #[serde(default)]
    pub description: Option<Extract>,
    #[serde(default)]
    pub thumbnail: Option<Extract>,
    #[serde(default)]
    pub genre: Option<Extract>,
    #[serde(default)]
    pub status: Option<Extract>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterRequest {
    pub url: String,
    #[serde(default)]
    pub json: bool,
    pub list: ChapterListSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterListSpec {
    pub item: String,
    pub name: Extract,
    pub url: Extract,
    #[serde(default)]
    pub date: Option<Extract>,
    #[serde(default)]
    pub scanlator: Option<Extract>,
    /// Overrides the number recognised from the chapter name.
    #[serde(default)]
    pub number: Option<Extract>,
    /// Items where this resolves to a value are dropped. Sources use it to hide
    /// chapters that are hosted elsewhere and cannot be read in-app.
    #[serde(default)]
    pub skip_if: Option<Extract>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRequest {
    pub url: String,
    #[serde(default)]
    pub json: bool,
    /// Selector-based extraction of the image list.
    #[serde(default)]
    pub list: Option<PageListSpec>,
    /// Alternative: pull every match out of the raw response body. Useful when
    /// the page list lives inside an inline script.
    #[serde(default)]
    pub regex: Option<RegexSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageListSpec {
    pub item: String,
    /// Where the image url sits inside each item; defaults to the item itself.
    #[serde(default)]
    pub image: Option<Extract>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilterOption {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FilterSpec {
    Header {
        name: String,
    },
    Separator,
    Text {
        name: String,
        param: String,
    },
    Select {
        name: String,
        #[serde(default)]
        param: Option<String>,
        options: Vec<FilterOption>,
    },
    Checkbox {
        name: String,
        /// Raw query fragment appended when ticked, e.g. `&status=completed`.
        query: String,
    },
    Tri {
        name: String,
        value: String,
        /// Templates receiving `{value}`.
        #[serde(default)]
        include: Option<String>,
        #[serde(default)]
        exclude: Option<String>,
    },
    Sort {
        name: String,
        param: String,
        options: Vec<FilterOption>,
        #[serde(default)]
        order_param: Option<String>,
    },
    Group {
        name: String,
        children: Vec<FilterSpec>,
    },
}

impl ExtensionManifest {
    pub fn parse(text: &str) -> Result<Self> {
        serde_json::from_str(text).context("the extension manifest is not valid")
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("parsing {}", path.display()))
    }

    fn to_filters(&self) -> FilterList {
        self.filters.iter().map(spec_to_filter).collect()
    }
}

fn spec_to_filter(spec: &FilterSpec) -> Filter {
    match spec {
        FilterSpec::Header { name } => Filter::header(name),
        FilterSpec::Separator => Filter::separator(),
        FilterSpec::Text { name, param } => Filter {
            name: name.clone(),
            kind: FilterKind::Text {
                value: String::new(),
            },
        }
        .tagged(param),
        FilterSpec::Select {
            name,
            param,
            options,
        } => Filter {
            name: name.clone(),
            kind: FilterKind::Select {
                options: options.iter().map(|o| o.label.clone()).collect(),
                values: options
                    .iter()
                    .map(|o| encode_param(param.as_deref(), &o.value))
                    .collect(),
                index: 0,
            },
        },
        FilterSpec::Checkbox { name, query } => Filter::checkbox(name, query),
        FilterSpec::Tri {
            name,
            value,
            include,
            exclude,
        } => Filter {
            name: name.clone(),
            kind: FilterKind::Tri {
                state: TriState::Disabled,
                // Both templates travel with the value, separated by a marker
                // the query builder splits back apart.
                value: format!(
                    "{}\u{1}{}\u{1}{}",
                    value,
                    include.clone().unwrap_or_default(),
                    exclude.clone().unwrap_or_default()
                ),
            },
        },
        FilterSpec::Sort {
            name,
            param,
            options,
            order_param,
        } => Filter {
            name: name.clone(),
            kind: FilterKind::Sort {
                options: options.iter().map(|o| o.label.clone()).collect(),
                values: options
                    .iter()
                    .map(|o| {
                        let base = encode_param(Some(param), &o.value);
                        match order_param {
                            Some(order) => format!("{base}\u{1}{order}"),
                            None => base,
                        }
                    })
                    .collect(),
                index: 0,
                ascending: false,
            },
        },
        FilterSpec::Group { name, children } => {
            Filter::group(name, children.iter().map(spec_to_filter).collect())
        }
    }
}

impl Filter {
    /// Carries the query parameter name alongside a text filter.
    fn tagged(mut self, param: &str) -> Self {
        self.name = format!("{}\u{1}{}", self.name, param);
        self
    }
}

fn encode_param(param: Option<&str>, value: &str) -> String {
    match param {
        Some(param) => format!("&{param}={}", urlencoding::encode(value)),
        None => value.to_string(),
    }
}

/// Splits a `name\u{1}param` pair produced by [`Filter::tagged`].
fn split_tag(text: &str) -> (&str, Option<&str>) {
    match text.split_once('\u{1}') {
        Some((left, right)) => (left, Some(right)),
        None => (text, None),
    }
}

/// Strips the parameter marker so the UI shows a clean label.
pub fn display_name(name: &str) -> &str {
    split_tag(name).0
}

// ---------------------------------------------------------------------------
// The source
// ---------------------------------------------------------------------------

pub struct ScriptedSource {
    id: i64,
    manifest: ExtensionManifest,
    http: Arc<HttpClient>,
}

impl ScriptedSource {
    pub fn new(manifest: ExtensionManifest, http: Arc<HttpClient>) -> Self {
        if let Some(limit) = &manifest.rate_limit
            && let Some(host) = crate::net::host_of(&manifest.base_url)
        {
            http.set_rate_limit(
                &host,
                limit.permits.max(1),
                Duration::from_millis(limit.period_ms.max(1)),
            );
        }
        Self {
            id: generate_id(&manifest.name, &manifest.lang, manifest.version),
            manifest,
            http,
        }
    }

    fn headers(&self) -> Vec<(String, String)> {
        self.manifest
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn build_url(&self, template: &str, page: u32, query: &str, item_url: &str) -> String {
        let id = item_url
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("");
        let offset = page.saturating_sub(1) * self.manifest.page_size.max(1);
        let resolved = template
            .replace("{base}", self.manifest.base_url.trim_end_matches('/'))
            .replace("{page}", &page.to_string())
            .replace("{page0}", &page.saturating_sub(1).to_string())
            .replace("{offset}", &offset.to_string())
            .replace("{limit}", &self.manifest.page_size.to_string())
            .replace("{query}", &urlencoding::encode(query))
            .replace("{url}", item_url)
            .replace("{id}", id);
        absolute_url(&self.manifest.base_url, &resolved)
    }

    /// Turns the live filter list into a query fragment.
    fn filters_to_query(&self, filters: &FilterList) -> String {
        let mut query = String::new();
        for_each_filter(filters, &mut |filter| match &filter.kind {
            FilterKind::Text { value } if !value.trim().is_empty() => {
                if let (_, Some(param)) = split_tag(&filter.name) {
                    query.push_str(&format!("&{param}={}", urlencoding::encode(value.trim())));
                }
            }
            FilterKind::Select { values, index, .. } => {
                if let Some(value) = values.get(*index)
                    && !value.is_empty()
                {
                    query.push_str(value);
                }
            }
            FilterKind::CheckBox { checked, value } if *checked => query.push_str(value),
            FilterKind::Tri { state, value } => {
                let mut parts = value.split('\u{1}');
                let raw = parts.next().unwrap_or_default();
                let include = parts.next().unwrap_or_default();
                let exclude = parts.next().unwrap_or_default();
                match state {
                    TriState::EnabledIs if !include.is_empty() => {
                        query.push_str(&include.replace("{value}", raw));
                    }
                    TriState::EnabledNot if !exclude.is_empty() => {
                        query.push_str(&exclude.replace("{value}", raw));
                    }
                    _ => {}
                }
            }
            FilterKind::Sort {
                values,
                index,
                ascending,
                ..
            } => {
                if let Some(value) = values.get(*index) {
                    let (fragment, order_param) = split_tag(value);
                    query.push_str(fragment);
                    if let Some(order_param) = order_param {
                        let direction = if *ascending { "asc" } else { "desc" };
                        query.push_str(&format!("&{order_param}={direction}"));
                    }
                }
            }
            _ => {}
        });
        query
    }

    async fn fetch_list(&self, request: &ListRequest, url: &str) -> Result<MangasPage> {
        let body = self.http.get_text(url, &self.headers()).await?;
        // Parsing is synchronous on purpose: the HTML tree is not `Send`, so it
        // must not be held across an await point.
        self.parse_list(request, &body)
    }

    fn parse_list(&self, request: &ListRequest, body: &str) -> Result<MangasPage> {
        let document = Document::parse(body, request.json)?;
        let root = document.root();
        let mut eval = Evaluator::new();

        let mut mangas = Vec::new();
        for item in eval.select_items(root, &request.list.item) {
            if let Some(skip) = &request.list.skip_if
                && eval.extract(item, skip).is_some()
            {
                continue;
            }
            let Some(title) = eval.extract(item, &request.list.title) else {
                continue;
            };
            let Some(url) = eval.extract(item, &request.list.url) else {
                continue;
            };
            let thumbnail = request
                .list
                .thumbnail
                .as_ref()
                .and_then(|spec| eval.extract(item, spec))
                .map(|t| absolute_url(&self.manifest.base_url, &t));

            let mut manga = SManga::new(relative_to_base(&self.manifest.base_url, &url), title);
            manga.thumbnail_url = thumbnail;
            manga.update_strategy = UpdateStrategy::AlwaysUpdate;
            mangas.push(manga);
        }

        let has_next_page = match &request.next_page {
            Some(spec) => eval
                .extract(root, spec)
                .map(|v| {
                    !matches!(
                        v.trim().to_lowercase().as_str(),
                        "" | "false" | "0" | "null"
                    )
                })
                .unwrap_or(false),
            None => !mangas.is_empty(),
        };

        Ok(MangasPage {
            mangas,
            has_next_page,
        })
    }
}

#[async_trait]
impl Source for ScriptedSource {
    fn id(&self) -> i64 {
        self.id
    }

    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn lang(&self) -> &str {
        &self.manifest.lang
    }

    fn base_url(&self) -> &str {
        &self.manifest.base_url
    }

    fn is_nsfw(&self) -> bool {
        self.manifest.nsfw
    }

    fn is_scripted(&self) -> bool {
        true
    }

    fn supports_latest(&self) -> bool {
        self.manifest.latest.is_some()
    }

    fn filters(&self) -> FilterList {
        self.manifest.to_filters()
    }

    fn image_headers(&self) -> Vec<(String, String)> {
        self.manifest
            .image_headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    async fn popular(&self, page: u32) -> Result<MangasPage> {
        let request = self
            .manifest
            .popular
            .as_ref()
            .context("this extension does not define a popular listing")?;
        let url = self.build_url(&request.url, page, "", "");
        self.fetch_list(request, &url).await
    }

    async fn latest(&self, page: u32) -> Result<MangasPage> {
        let request = self
            .manifest
            .latest
            .as_ref()
            .context("this extension does not define a latest listing")?;
        let url = self.build_url(&request.url, page, "", "");
        self.fetch_list(request, &url).await
    }

    async fn search(&self, page: u32, query: &str, filters: &FilterList) -> Result<MangasPage> {
        let request = self
            .manifest
            .search
            .as_ref()
            .or(self.manifest.popular.as_ref())
            .context("this extension does not define a search")?;
        let url = self
            .build_url(&request.url, page, query, "")
            .replace("{filters}", &self.filters_to_query(filters));
        // The placeholder may survive `build_url` when it sits in the query.
        let url = url.replace("%7Bfilters%7D", "");
        self.fetch_list(request, &url).await
    }

    async fn details(&self, manga: &SManga) -> Result<SManga> {
        let request = &self.manifest.details;
        let url = self.build_url(&request.url, 1, "", &manga.url);
        let body = self.http.get_text(&url, &self.headers()).await?;
        self.parse_details(manga, &body)
    }

    async fn chapters(&self, manga: &SManga) -> Result<Vec<SChapter>> {
        let request = &self.manifest.chapters;
        let url = self.build_url(&request.url, 1, "", &manga.url);
        let body = self.http.get_text(&url, &self.headers()).await?;
        self.parse_chapters(manga, &body)
    }

    async fn pages(&self, _manga: &SManga, chapter: &SChapter) -> Result<Vec<Page>> {
        let request = &self.manifest.pages;
        let url = self.build_url(&request.url, 1, "", &chapter.url);
        let body = self.http.get_text(&url, &self.headers()).await?;
        self.parse_pages(&body)
    }
}

impl ScriptedSource {
    fn parse_details(&self, manga: &SManga, body: &str) -> Result<SManga> {
        let request = &self.manifest.details;
        let document = Document::parse(body, request.json)?;
        let mut eval = Evaluator::new();

        let root = match &request.root {
            Some(path) => eval
                .select_items(document.root(), path)
                .into_iter()
                .next()
                .unwrap_or(document.root()),
            None => document.root(),
        };

        let fields = &request.fields;
        let genre = fields
            .genre
            .as_ref()
            .map(|spec| eval.extract_all(root, spec))
            .filter(|list| !list.is_empty());

        let status = fields
            .status
            .as_ref()
            .and_then(|spec| eval.extract(root, spec))
            .map(|value| parse_status(&value))
            .unwrap_or(MangaStatus::Unknown);

        Ok(SManga {
            url: manga.url.clone(),
            title: eval
                .extract_or_empty(root, &fields.title)
                .unwrap_or_else(|| manga.title.clone()),
            author: eval.extract_or_empty(root, &fields.author),
            artist: eval.extract_or_empty(root, &fields.artist),
            description: eval.extract_or_empty(root, &fields.description),
            thumbnail_url: eval
                .extract_or_empty(root, &fields.thumbnail)
                .map(|t| absolute_url(&self.manifest.base_url, &t))
                .or_else(|| manga.thumbnail_url.clone()),
            genre,
            status,
            update_strategy: UpdateStrategy::AlwaysUpdate,
            initialized: true,
        })
    }

    fn parse_chapters(&self, manga: &SManga, body: &str) -> Result<Vec<SChapter>> {
        let request = &self.manifest.chapters;
        let document = Document::parse(body, request.json)?;
        let mut eval = Evaluator::new();
        let spec = &request.list;

        let mut chapters = Vec::new();
        for item in eval.select_items(document.root(), &spec.item) {
            if let Some(skip) = &spec.skip_if
                && eval.extract(item, skip).is_some()
            {
                continue;
            }
            let Some(name) = eval.extract(item, &spec.name) else {
                continue;
            };
            let Some(url) = eval.extract(item, &spec.url) else {
                continue;
            };
            let number = spec
                .number
                .as_ref()
                .and_then(|s| eval.extract(item, s))
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or_else(|| super::recognise_chapter_number(&manga.title, &name));

            chapters.push(SChapter {
                url: relative_to_base(&self.manifest.base_url, &url),
                name,
                date_upload: eval.extract_date(item, &spec.date),
                chapter_number: number,
                scanlator: eval.extract_or_empty(item, &spec.scanlator),
            });
        }

        if chapters.is_empty() {
            bail!("no chapters matched the extension's selectors");
        }
        Ok(chapters)
    }

    fn parse_pages(&self, body: &str) -> Result<Vec<Page>> {
        let request = &self.manifest.pages;
        let headers = self.image_headers();

        // Regex mode reads the raw body, for sites that inline the page list.
        if let Some(spec) = &request.regex {
            let regex = regex::Regex::new(&spec.pattern)
                .with_context(|| format!("invalid page regex {:?}", spec.pattern))?;
            let urls: Vec<String> = regex
                .captures_iter(body)
                .filter_map(|caps| caps.get(1).or_else(|| caps.get(0)))
                .map(|m| m.as_str().replace("\\/", "/"))
                .collect();
            if urls.is_empty() {
                bail!("the page regex matched nothing");
            }
            return Ok(urls
                .into_iter()
                .enumerate()
                .map(|(index, url)| Page {
                    index,
                    image_url: absolute_url(&self.manifest.base_url, &url),
                    fallbacks: Vec::new(),
                    headers: headers.clone(),
                })
                .collect());
        }

        let spec = request
            .list
            .as_ref()
            .context("the extension defines neither a page list nor a page regex")?;
        let document = Document::parse(body, request.json)?;
        let mut eval = Evaluator::new();
        let root = document.root();

        let mut pages = Vec::new();
        for item in eval.select_items(root, &spec.item) {
            let url = match &spec.image {
                // Page urls often combine a document-level base with a
                // per-item file name, so the root stays reachable here.
                Some(extract) => eval.extract_in(item, root, extract),
                None => default_image_value(&mut eval, item),
            };
            if let Some(url) = url {
                pages.push(Page {
                    index: pages.len(),
                    image_url: absolute_url(&self.manifest.base_url, &url),
                    fallbacks: Vec::new(),
                    headers: headers.clone(),
                });
            }
        }

        if pages.is_empty() {
            bail!("no pages matched the extension's selectors");
        }
        Ok(pages)
    }
}

/// When no image spec is given, try the attributes lazy-loading sites use.
fn default_image_value(eval: &mut Evaluator, node: Node<'_>) -> Option<String> {
    for attr in ["src", "data-src", "data-original", "data-lazy-src"] {
        let spec = Extract::Spec(Box::new(eval::ExtractSpec {
            attr: Some(attr.to_string()),
            ..Default::default()
        }));
        if let Some(value) = eval.extract(node, &spec) {
            return Some(value);
        }
    }
    None
}

fn parse_status(value: &str) -> MangaStatus {
    let lowered = value.to_lowercase();
    if lowered.contains("ongoing")
        || lowered.contains("releasing")
        || lowered.contains("publishing")
    {
        MangaStatus::Ongoing
    } else if lowered.contains("completed") || lowered.contains("finished") {
        MangaStatus::Completed
    } else if lowered.contains("hiatus") {
        MangaStatus::OnHiatus
    } else if lowered.contains("cancel") || lowered.contains("dropped") {
        MangaStatus::Cancelled
    } else if lowered.contains("licensed") {
        MangaStatus::Licensed
    } else {
        MangaStatus::Unknown
    }
}

/// Stores urls relative to the base so a domain change does not break a library.
fn relative_to_base(base: &str, url: &str) -> String {
    let base = base.trim_end_matches('/');
    match url.strip_prefix(base) {
        Some(rest) if rest.starts_with('/') => rest.to_string(),
        _ => url.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Installation
// ---------------------------------------------------------------------------

/// One entry of a remote repository index.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoEntry {
    pub id: String,
    pub name: String,
    pub lang: String,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub nsfw: bool,
    #[serde(default)]
    pub description: Option<String>,
    /// Absolute url of the manifest file.
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct InstalledExtension {
    pub manifest: ExtensionManifest,
    pub path: PathBuf,
    pub source_id: i64,
}

/// Reads every manifest in `dir`, skipping (and logging) the broken ones.
pub fn load_installed(dir: &Path) -> Vec<InstalledExtension> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut installed = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match ExtensionManifest::load(&path) {
            Ok(manifest) => {
                let source_id = generate_id(&manifest.name, &manifest.lang, manifest.version);
                installed.push(InstalledExtension {
                    manifest,
                    path,
                    source_id,
                });
            }
            Err(err) => log::warn!("skipping extension {}: {err:#}", path.display()),
        }
    }
    installed.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    installed
}

/// Downloads a repository index.
pub async fn fetch_repo(http: &HttpClient, repo_url: &str) -> Result<Vec<RepoEntry>> {
    let bytes = http.get_bytes(repo_url, &[]).await?;

    if let Some(reason) = describe_incompatible_index(repo_url, &bytes) {
        bail!("{reason}");
    }

    let text = String::from_utf8(bytes)
        .context("the repository index is not text; it may be a binary Mihon index")?;
    let entries: Vec<RepoEntry> = serde_json::from_str(&text)
        .context("the repository index is not a valid extension list")?;
    Ok(entries)
}

/// Recognises index formats that cannot work here and explains why.
///
/// Anyone arriving from Mihon will paste an Android extension repository first,
/// and "expected value at line 1" would tell them nothing.
fn describe_incompatible_index(url: &str, bytes: &[u8]) -> Option<String> {
    const ANDROID_REPO: &str = "This is a Mihon/Tachiyomi repository for Android extensions. \
         Those are compiled Android apps (.apk) and cannot run in a desktop build. \
         Use a scripted extension (a .json manifest) instead — see the example in the \
         app's extensions folder.";

    let path = url.split(['?', '#']).next().unwrap_or(url);
    if path.ends_with(".pb") || path.ends_with(".pb.gz") {
        return Some(ANDROID_REPO.to_string());
    }
    // Gzip magic: the protobuf index is served compressed.
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return Some(ANDROID_REPO.to_string());
    }
    // The JSON variant parses, but every entry describes an apk.
    if let Ok(text) = std::str::from_utf8(bytes)
        && let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(text)
    {
        let android = values.iter().take(5).any(|entry| {
            entry.get("apk").is_some()
                || entry
                    .get("pkg")
                    .and_then(|v| v.as_str())
                    .map(|pkg| pkg.contains("tachiyomi.extension"))
                    .unwrap_or(false)
        });
        if android {
            return Some(ANDROID_REPO.to_string());
        }
    }
    None
}

/// Downloads a manifest, validates it, and writes it into `dir`.
pub async fn install(http: &HttpClient, dir: &Path, entry: &RepoEntry) -> Result<PathBuf> {
    let text = http.get_text(&entry.url, &[]).await?;
    // Parse before writing so a broken download never lands on disk.
    let manifest = ExtensionManifest::parse(&text)?;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", sanitise(&manifest.id)));
    std::fs::write(&path, &text).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

pub fn uninstall(path: &Path) -> Result<()> {
    std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))
}

/// Installs from a file the user picked locally.
pub fn install_from_file(dir: &Path, file: &Path) -> Result<PathBuf> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let manifest = ExtensionManifest::parse(&text)?;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", sanitise(&manifest.id)));
    std::fs::write(&path, &text)?;
    Ok(path)
}

fn sanitise(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
        "id": "demo",
        "name": "Demo",
        "lang": "en",
        "baseUrl": "https://demo.test",
        "popular": {
            "url": "{base}/popular?page={page}",
            "list": {
                "item": "div.item",
                "title": "a.title",
                "url": {"selector": "a.title", "attr": "href"},
                "thumbnail": {"selector": "img", "attr": "src"}
            },
            "nextPage": "a.next"
        },
        "details": {
            "url": "{base}{url}",
            "fields": {
                "title": "h1",
                "description": "div.summary",
                "status": {"selector": "span.status"}
            }
        },
        "chapters": {
            "url": "{base}{url}",
            "list": {"item": "li.ch", "name": "a", "url": {"selector": "a", "attr": "href"}}
        },
        "pages": {
            "url": "{base}{url}",
            "list": {"item": "img.page", "image": {"attr": "src"}}
        }
    }"#;

    fn source() -> ScriptedSource {
        let manifest = ExtensionManifest::parse(MANIFEST).unwrap();
        let http = Arc::new(HttpClient::new().unwrap());
        ScriptedSource::new(manifest, http)
    }

    #[test]
    fn manifest_parses() {
        let manifest = ExtensionManifest::parse(MANIFEST).unwrap();
        assert_eq!(manifest.name, "Demo");
        assert!(manifest.popular.is_some());
        assert!(manifest.latest.is_none());
    }

    #[test]
    fn url_templates_are_filled_in() {
        let source = source();
        let url = source.build_url("{base}/popular?page={page}", 3, "", "");
        assert_eq!(url, "https://demo.test/popular?page=3");
        let url = source.build_url("{base}{url}", 1, "", "/manga/42");
        assert_eq!(url, "https://demo.test/manga/42");
    }

    #[test]
    fn list_parsing_reads_items_and_next_page() {
        let source = source();
        let request = source.manifest.popular.clone().unwrap();
        let body = r#"<html><body>
            <div class="item"><a class="title" href="/manga/1">First</a><img src="/c1.png"></div>
            <div class="item"><a class="title" href="/manga/2">Second</a><img src="/c2.png"></div>
            <a class="next" href="?page=2">Next</a>
        </body></html>"#;
        let page = source.parse_list(&request, body).unwrap();
        assert_eq!(page.mangas.len(), 2);
        assert_eq!(page.mangas[0].title, "First");
        assert_eq!(page.mangas[0].url, "/manga/1");
        assert_eq!(
            page.mangas[0].thumbnail_url.as_deref(),
            Some("https://demo.test/c1.png")
        );
        assert!(page.has_next_page);
    }

    #[test]
    fn details_and_pages_parse() {
        let source = source();
        let manga = SManga::new("/manga/1", "First");
        let details = source
            .parse_details(
                &manga,
                r#"<h1>Real Title</h1><div class="summary">Blurb</div><span class="status">Ongoing</span>"#,
            )
            .unwrap();
        assert_eq!(details.title, "Real Title");
        assert_eq!(details.status, MangaStatus::Ongoing);

        let pages = source
            .parse_pages(r#"<img class="page" src="/p1.jpg"><img class="page" src="/p2.jpg">"#)
            .unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[1].image_url, "https://demo.test/p2.jpg");
    }

    #[test]
    fn missing_selectors_surface_as_errors() {
        let source = source();
        assert!(source.parse_pages("<div>nothing here</div>").is_err());
    }

    /// The shipped example must stay loadable and correct: it doubles as the
    /// reference documentation for the format.
    fn example_manifest() -> ExtensionManifest {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/extensions/mangadex-scripted-en.json");
        ExtensionManifest::load(&path).expect("the shipped example must parse")
    }

    #[test]
    fn android_repositories_are_recognised_and_explained() {
        let expect_android = |reason: Option<String>| {
            let reason = reason.expect("should be recognised as an Android repository");
            assert!(reason.contains("Android"), "unhelpful message: {reason}");
            assert!(reason.contains(".json"), "should point at the alternative");
        };

        // The protobuf index, by extension and by gzip magic.
        expect_android(describe_incompatible_index(
            "https://github.com/keiyoushi/extensions/raw/repo/index.pb",
            b"",
        ));
        expect_android(describe_incompatible_index(
            "https://example.com/index",
            &[0x1f, 0x8b, 0x08, 0x00],
        ));

        // The JSON variant, whose entries describe apks.
        let mihon_json = br#"[{"name":"Some Source","pkg":"eu.kanade.tachiyomi.extension.en.x",
                              "apk":"tachiyomi-en.x-v1.4.1.apk","lang":"en","code":1}]"#;
        expect_android(describe_incompatible_index(
            "https://x/index.min.json",
            mihon_json,
        ));

        // A genuine scripted index must pass through untouched.
        let ours = br#"[{"id":"demo","name":"Demo","lang":"en","url":"https://x/demo.json"}]"#;
        assert!(describe_incompatible_index("https://x/index.json", ours).is_none());
    }

    /// The exact url a Mihon user is most likely to paste.
    #[test]
    #[ignore]
    fn live_android_repo_is_refused_clearly() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let http = HttpClient::new().unwrap();
            let err = fetch_repo(
                &http,
                "https://github.com/keiyoushi/extensions/raw/repo/index.pb",
            )
            .await
            .expect_err("an Android repository must be refused");
            let message = format!("{err:#}");
            println!("refused with: {message}");
            assert!(message.contains("Android"), "unhelpful: {message}");
        });
    }

    #[test]
    fn shipped_example_parses_and_declares_everything() {
        let manifest = example_manifest();
        assert_eq!(manifest.lang, "en");
        assert!(manifest.popular.is_some());
        assert!(manifest.latest.is_some());
        assert!(manifest.search.is_some());
        assert_eq!(manifest.page_size, 24);
    }

    #[test]
    fn shipped_example_parses_a_recorded_response() {
        let manifest = example_manifest();
        let http = Arc::new(HttpClient::new().unwrap());
        let source = ScriptedSource::new(manifest, http);

        // A trimmed-down copy of a real /manga response.
        let body = r#"{
            "data": [{
                "id": "abc-123",
                "attributes": {"title": {"en": "Test Manga"}},
                "relationships": [
                    {"type": "author", "attributes": {"name": "An Author"}},
                    {"type": "cover_art", "attributes": {"fileName": "cover.jpg"}}
                ]
            }],
            "limit": 24, "offset": 0, "total": 100
        }"#;

        let request = source.manifest.popular.clone().unwrap();
        let page = source.parse_list(&request, body).unwrap();
        assert_eq!(page.mangas.len(), 1);
        assert_eq!(page.mangas[0].title, "Test Manga");
        assert_eq!(page.mangas[0].url, "/manga/abc-123");
        // Composed from two separate fields of the same record.
        assert_eq!(
            page.mangas[0].thumbnail_url.as_deref(),
            Some("https://uploads.mangadex.org/covers/abc-123/cover.jpg.512.jpg")
        );
        assert!(page.has_next_page);
    }

    #[test]
    fn shipped_example_builds_page_urls_from_the_document_root() {
        let manifest = example_manifest();
        let http = Arc::new(HttpClient::new().unwrap());
        let source = ScriptedSource::new(manifest, http);

        let body = r#"{
            "baseUrl": "https://cdn.example.test",
            "chapter": {"hash": "deadbeef", "data": ["p1.jpg", "p2.jpg"]}
        }"#;
        let pages = source.parse_pages(body).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(
            pages[0].image_url,
            "https://cdn.example.test/data/deadbeef/p1.jpg"
        );
        assert_eq!(
            pages[1].image_url,
            "https://cdn.example.test/data/deadbeef/p2.jpg"
        );
    }

    #[test]
    fn shipped_example_reads_chapter_metadata() {
        let manifest = example_manifest();
        let http = Arc::new(HttpClient::new().unwrap());
        let source = ScriptedSource::new(manifest, http);

        let body = r#"{"data": [
            {"id": "c1", "attributes": {"chapter": "12", "title": "The Title",
             "publishAt": "2021-03-04T05:06:07+00:00"},
             "relationships": [{"type": "scanlation_group",
                                "attributes": {"name": "Some Group"}}]},
            {"id": "c2", "attributes": {"chapter": "13", "title": null,
             "publishAt": "2021-04-04T05:06:07+00:00"}, "relationships": []}
        ]}"#;

        let manga = SManga::new("/manga/x", "Test");
        let chapters = source.parse_chapters(&manga, body).unwrap();
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].name, "Ch.12 - The Title");
        assert_eq!(chapters[0].chapter_number, 12.0);
        assert_eq!(chapters[0].scanlator.as_deref(), Some("Some Group"));
        // A missing title must not swallow the whole name.
        assert_eq!(chapters[1].name, "Ch.13");
        assert!(chapters[1].date_upload > 0);
    }

    /// Runs the shipped example against the live API.
    #[test]
    #[ignore]
    fn shipped_example_works_against_the_live_api() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let http = Arc::new(HttpClient::new().unwrap());
            let source = ScriptedSource::new(example_manifest(), http);

            let popular = source.popular(1).await.expect("popular failed");
            assert!(!popular.mangas.is_empty());
            println!("scripted popular[0] = {}", popular.mangas[0].title);
            assert!(
                popular.mangas[0]
                    .thumbnail_url
                    .as_deref()
                    .unwrap_or_default()
                    .starts_with("https://uploads.mangadex.org/covers/")
            );

            let details = source
                .details(&popular.mangas[0])
                .await
                .expect("details failed");
            assert!(!details.title.is_empty());
            println!("scripted details: {} ({:?})", details.title, details.status);

            let mut readable = None;
            for candidate in popular.mangas.iter().take(6) {
                let chapters = source.chapters(candidate).await.unwrap_or_default();
                if !chapters.is_empty() {
                    readable = Some((candidate.clone(), chapters));
                    break;
                }
            }
            let (manga, chapters) = readable.expect("no readable chapters found");
            println!("scripted chapters: {}", chapters.len());

            let pages = source
                .pages(&manga, chapters.last().unwrap())
                .await
                .expect("pages failed");
            assert!(!pages.is_empty());
            assert!(pages[0].image_url.starts_with("http"));
            println!("scripted pages: {} -> {}", pages.len(), pages[0].image_url);
        });
    }
}
