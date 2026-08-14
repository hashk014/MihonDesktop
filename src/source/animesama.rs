//! Anime-Sama (French), ported from the Kotlin extension.
//!
//! This source cannot be expressed as a scripted manifest: building a chapter
//! list means following a chain of requests and interpreting inline JavaScript
//! (`panneauScan`, `creerListe`, `newSP`) rather than reading fixed selectors.
//! The logic below mirrors the upstream extension step for step.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use parking_lot::RwLock;
use scraper::{Html, Selector};

use super::{
    Filter, FilterKind, FilterList, MangasPage, Page, SChapter, SManga, Source, for_each_filter,
    generate_id,
};
use crate::model::{MangaStatus, UpdateStrategy};
use crate::net::{HttpClient, absolute_url};

const BASE: &str = "https://anime-sama.to";
/// Matches the upstream extension's versionCode, so ids stay comparable.
const VERSION: u32 = 17;

pub struct AnimeSama {
    id: i64,
    name: String,
    http: Arc<HttpClient>,
    /// Genres are only listed on the catalogue page, so they are cached the
    /// first time one is fetched rather than requested up front.
    genres: RwLock<Vec<(String, String)>>,
}

impl AnimeSama {
    pub fn new(http: Arc<HttpClient>) -> Self {
        // The site is a small operation; keep the request rate polite.
        http.set_rate_limit("anime-sama.to", 2, Duration::from_secs(1));
        let name = "AnimeSama".to_string();
        Self {
            id: generate_id(&name, "fr", VERSION),
            name,
            http,
            genres: RwLock::new(Vec::new()),
        }
    }

    fn headers(&self) -> Vec<(String, String)> {
        vec![("Accept-Language".into(), "fr-FR".into())]
    }

    async fn catalogue(&self, page: u32, query: &str, filters: &FilterList) -> Result<MangasPage> {
        let mut url = format!("{BASE}/catalogue?type%5B%5D=Scans&page={page}");
        if !query.trim().is_empty() {
            url.push_str(&format!("&search={}", urlencoding::encode(query.trim())));
        }
        for_each_filter(filters, &mut |filter| {
            if let FilterKind::CheckBox { checked, value } = &filter.kind
                && *checked
            {
                url.push_str(&format!("&genre%5B%5D={}", urlencoding::encode(value)));
            }
        });

        let html = self.http.get_text(&url, &self.headers()).await?;
        // Parsing is synchronous: the HTML tree is not `Send`.
        let (page, genres) = parse_catalogue(&html);
        if !genres.is_empty() {
            *self.genres.write() = genres;
        }
        Ok(page)
    }
}

#[async_trait]
impl Source for AnimeSama {
    fn id(&self) -> i64 {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn lang(&self) -> &str {
        "fr"
    }

    fn base_url(&self) -> &str {
        BASE
    }

    fn filters(&self) -> FilterList {
        let genres = self.genres.read().clone();
        if genres.is_empty() {
            return vec![Filter::header("Browse once to load the genre list")];
        }
        vec![Filter::group(
            "Genres",
            genres
                .iter()
                .map(|(label, value)| Filter::checkbox(label, value))
                .collect(),
        )]
    }

    fn image_headers(&self) -> Vec<(String, String)> {
        // The image host refuses requests without a referer.
        vec![
            ("Referer".into(), format!("{BASE}/")),
            ("Accept-Language".into(), "fr-FR".into()),
        ]
    }

    async fn popular(&self, page: u32) -> Result<MangasPage> {
        self.catalogue(page, "", &Vec::new()).await
    }

    async fn latest(&self, _page: u32) -> Result<MangasPage> {
        let html = self.http.get_text(BASE, &self.headers()).await?;
        Ok(parse_latest(&html))
    }

    async fn search(&self, page: u32, query: &str, filters: &FilterList) -> Result<MangasPage> {
        self.catalogue(page, query, filters).await
    }

    async fn details(&self, manga: &SManga) -> Result<SManga> {
        let url = absolute_url(BASE, &manga.url);
        let html = self.http.get_text(&url, &self.headers()).await?;
        let mut details = parse_details(&html, &url);
        details.url = manga.url.clone();
        details.initialized = true;
        Ok(details)
    }

    async fn chapters(&self, manga: &SManga) -> Result<Vec<SChapter>> {
        let manga_url = absolute_url(BASE, &manga.url);
        let html = self.http.get_text(&manga_url, &self.headers()).await?;

        let mut chapters: Vec<SChapter> = Vec::new();

        for (panel_title, panel_url) in parse_scan_panels(&html) {
            // "va" marks the English release, which this source does not host.
            if panel_url.contains("va") {
                continue;
            }
            let scanlator = clean_scanlator(&panel_title);
            let sub_url = join_path(&manga_url, &panel_url);

            let Ok(sub_html) = self.http.get_text(&sub_url, &self.headers()).await else {
                continue;
            };

            // The work title drives both the API lookup and the image paths. It
            // sometimes only exists on the linked "main" page of the series.
            let (main_link, own_title) = parse_sub_page(&sub_html, &sub_url);
            let mut work_title = String::new();
            if let Some(link) = main_link
                && let Ok(main_html) = self.http.get_text(&link, &self.headers()).await
            {
                work_title = parse_work_title(&main_html);
            }
            if work_title.trim().is_empty() {
                work_title = own_title;
            }
            if work_title.trim().is_empty() {
                continue;
            }

            let api_url = format!(
                "{BASE}/s2/scans/get_nb_chap_et_img.php?oeuvre={}",
                urlencoding::encode(&work_title)
            );
            // The response is either a {chapter: pageCount} map or an error
            // object. Falling back to an empty map silently drops every chapter
            // the page's script did not name, so it is worth a warning.
            let counts: BTreeMap<String, i64> = match self
                .http
                .get_json(&api_url, &self.headers())
                .await
                .map(serde_json::from_value::<BTreeMap<String, i64>>)
            {
                Ok(Ok(counts)) => counts,
                Ok(Err(_)) => {
                    log::warn!(
                        "anime-sama: no chapter count for {work_title:?}; \
                         the list will only contain the chapters named in the page"
                    );
                    BTreeMap::new()
                }
                Err(err) => {
                    log::warn!("anime-sama: chapter count request failed: {err:#}");
                    BTreeMap::new()
                }
            };

            for (name, index) in build_chapter_names(&sub_html, counts.len()) {
                chapters.push(SChapter {
                    url: format!(
                        "/s2/scans/get_nb_chap_et_img.php?oeuvre={}&id={index}&title={}",
                        urlencoding::encode(&work_title),
                        urlencoding::encode(&work_title)
                    ),
                    name: format!("Chapitre {name}"),
                    chapter_number: name.parse::<f64>().unwrap_or(-1.0),
                    date_upload: 0,
                    scanlator: (!scanlator.is_empty()).then(|| scanlator.clone()),
                });
            }
        }

        // Upstream keeps the first occurrence of each name, orders by id and
        // then presents the newest first.
        let mut seen = std::collections::HashSet::new();
        chapters.retain(|c| seen.insert(c.name.clone()));
        chapters.sort_by_key(|c| query_param(&c.url, "id").and_then(|v| v.parse::<i64>().ok()));
        chapters.reverse();

        Ok(chapters)
    }

    async fn pages(&self, _manga: &SManga, chapter: &SChapter) -> Result<Vec<Page>> {
        let url = absolute_url(BASE, &chapter.url);
        let title = query_param(&chapter.url, "oeuvre")
            .map(|v| percent_decode(&v))
            .context("chapter url has no work title")?;
        let id = query_param(&chapter.url, "id").context("chapter url has no id")?;

        let json = self.http.get_json(&url, &self.headers()).await?;
        let counts: BTreeMap<String, i64> =
            serde_json::from_value(json).context("unexpected page-count response")?;
        let count = counts.get(&id).copied().unwrap_or(0);
        if count <= 0 {
            anyhow::bail!("this chapter has no pages yet");
        }

        let headers = self.image_headers();
        Ok((1..=count)
            .map(|index| Page {
                index: (index - 1) as usize,
                image_url: format!("{BASE}/s2/scans/{}/{id}/{index}.jpg", encode_path(&title)),
                fallbacks: Vec::new(),
                headers: headers.clone(),
            })
            .collect())
    }

    fn web_url(&self, manga: &SManga) -> String {
        absolute_url(BASE, &manga.url)
    }
}

// ---------------------------------------------------------------------------
// Parsing (synchronous: `Html` is not `Send`)
// ---------------------------------------------------------------------------

fn select(css: &str) -> Option<Selector> {
    Selector::parse(css).ok()
}

/// Catalogue listing plus the genre checkboxes shown alongside it.
fn parse_catalogue(html: &str) -> (MangasPage, Vec<(String, String)>) {
    let document = Html::parse_document(html);
    let mangas = collect_cards(&document, "div#list_catalog > div", "");

    let has_next_page = select("#list_pagination > a.bg-sky-900 + a")
        .map(|s| document.select(&s).next().is_some())
        .unwrap_or(false);

    let mut genres = Vec::new();
    if let (Some(labels), Some(input), Some(span)) = (
        select("#list_genres #genreList label"),
        select("input[name='genre[]']"),
        select("span"),
    ) {
        for label in document.select(&labels) {
            let value = label
                .select(&input)
                .next()
                .and_then(|i| i.value().attr("value"));
            let text = label.select(&span).next().map(|s| text_of(s));
            if let (Some(value), Some(text)) = (value, text)
                && !text.is_empty()
            {
                genres.push((text, value.to_string()));
            }
        }
    }

    (
        MangasPage {
            mangas,
            has_next_page,
        },
        genres,
    )
}

/// The home page lists freshly added scans; their links point at the reader.
fn parse_latest(html: &str) -> MangasPage {
    let document = Html::parse_document(html);
    MangasPage {
        mangas: collect_cards(&document, "div#containerAjoutsScans > div", "scan/vf/"),
        has_next_page: false,
    }
}

fn collect_cards(document: &Html, container: &str, strip_suffix: &str) -> Vec<SManga> {
    let (Some(items), Some(title_sel), Some(link_sel), Some(img_sel)) = (
        select(container),
        select("h2.card-title"),
        select("a"),
        select("img"),
    ) else {
        return Vec::new();
    };

    document
        .select(&items)
        .filter_map(|item| {
            let title = item.select(&title_sel).next().map(|t| text_of(t))?;
            let href = item
                .select(&link_sel)
                .next()
                .and_then(|a| a.value().attr("href"))?;
            let mut url = absolute_url(BASE, href);
            if !strip_suffix.is_empty() {
                url = url.trim_end_matches(strip_suffix).to_string();
            }

            let thumbnail = item
                .select(&img_sel)
                .next()
                .and_then(|i| i.value().attr("src"))
                .map(|src| absolute_url(BASE, src));

            let mut manga = SManga::new(relative(&url), title);
            manga.thumbnail_url = thumbnail;
            manga.update_strategy = UpdateStrategy::AlwaysUpdate;
            Some(manga)
        })
        .collect()
}

fn parse_details(html: &str, url: &str) -> SManga {
    let document = Html::parse_document(html);

    let title = select("div.my-2 h1")
        .and_then(|s| document.select(&s).next().map(|e| text_of(e)))
        .unwrap_or_default();
    let description = select("p#synopsisText")
        .and_then(|s| document.select(&s).next().map(|e| text_of(e)))
        .filter(|d| !d.is_empty());
    let thumbnail_url = select("img#coverOeuvre")
        .and_then(|s| document.select(&s).next())
        .and_then(|e| e.value().attr("src"))
        .map(|src| absolute_url(BASE, src));

    let genre: Vec<String> = select("span.genre-pill")
        .map(|s| document.select(&s).map(|e| text_of(e)).collect())
        .unwrap_or_default();

    // `:contains()` is a jsoup extension, so the label/value pairs are walked
    // by hand here.
    let author = info_value(&document, "Créateur");
    let status = match info_value(&document, "État")
        .map(|s| s.to_lowercase())
        .as_deref()
    {
        Some("en cours") => MangaStatus::Ongoing,
        Some("terminé") => MangaStatus::Completed,
        _ => MangaStatus::Unknown,
    };

    SManga {
        url: relative(url),
        title,
        artist: None,
        author,
        description,
        genre: (!genre.is_empty()).then_some(genre),
        status,
        thumbnail_url,
        update_strategy: UpdateStrategy::AlwaysUpdate,
        initialized: true,
    }
}

/// Finds `<span class="info-lbl">Label</span><span class="info-val">Value</span>`.
fn info_value(document: &Html, label: &str) -> Option<String> {
    let labels = select("span.info-lbl")?;
    for element in document.select(&labels) {
        if !text_of(element).contains(label) {
            continue;
        }
        let sibling = element
            .next_siblings()
            .filter_map(scraper::ElementRef::wrap)
            .find(|e| {
                e.value()
                    .has_class("info-val", scraper::CaseSensitivity::AsciiCaseInsensitive)
            });
        if let Some(value) = sibling {
            let text = text_of(value);
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// Extracts the `panneauScan("name", "url")` calls from the inline script.
fn parse_scan_panels(html: &str) -> Vec<(String, String)> {
    let document = Html::parse_document(html);
    let Some(scripts) = select("script") else {
        return Vec::new();
    };

    let bodies: Vec<String> = document.select(&scripts).map(|s| s.inner_html()).collect();
    // Prefer the script carrying the template call, as upstream does, but fall
    // back to any script that calls the function at all.
    let script = bodies
        .iter()
        .find(|body| body.contains("panneauScan(\"nom\", \"url\")"))
        .or_else(|| bodies.iter().find(|body| body.contains("panneauScan(")));
    let Some(script) = script else {
        return Vec::new();
    };

    let Ok(regex) = regex::Regex::new(r#"panneauScan\("(.+?)",\s*"(.+?)"\)"#) else {
        return Vec::new();
    };

    regex
        .captures_iter(script)
        .filter_map(|caps| {
            Some((
                caps.get(1)?.as_str().to_string(),
                caps.get(2)?.as_str().to_string(),
            ))
        })
        // Drop the literal template call. Upstream skips it by position, which
        // would silently eat a real entry if the page ever reordered.
        .filter(|(name, url)| !(name == "nom" && url == "url"))
        .collect()
}

/// Returns the optional link to the series' main page, and the title on this page.
fn parse_sub_page(html: &str, base: &str) -> (Option<String>, String) {
    let document = Html::parse_document(html);
    let link = select("a:has(#imgOeuvre.grayscale)")
        .and_then(|s| document.select(&s).next())
        .and_then(|a| a.value().attr("href"))
        .map(|href| absolute_url(base, href));
    (link, work_title(&document))
}

fn parse_work_title(html: &str) -> String {
    work_title(&Html::parse_document(html))
}

/// `#titreOeuvre`'s first text node — later nodes hold the alternative titles.
///
/// The value is deliberately **not trimmed**: the site reads this element with
/// `innerHTML` and the chapter API keys on it byte for byte. Several works are
/// stored with a trailing space, and trimming makes the API answer
/// "Oeuvre not found", silently truncating the chapter list.
fn work_title(document: &Html) -> String {
    let Some(selector) = select("#titreOeuvre") else {
        return String::new();
    };
    document
        .select(&selector)
        .next()
        .and_then(|e| {
            e.children()
                .filter_map(|node| node.value().as_text().map(|t| t.to_string()))
                .next()
        })
        .unwrap_or_default()
}

/// Replays the reader's list-building script to recover the chapter names.
///
/// `creerListe(a, b)` adds the range a..=b, `newSP(x)` adds one special. Any
/// chapter the API knows about but the script did not name is numbered
/// sequentially, offset by the specials already emitted.
fn build_chapter_names(sub_html: &str, api_count: usize) -> Vec<(String, usize)> {
    let mut names: Vec<String> = Vec::new();
    let mut specials = 0usize;

    if sub_html.contains("resetListe()") {
        let create = regex::Regex::new(r"creerListe\((\d+)\s*,\s*(\d+)\)").ok();
        let special = regex::Regex::new(r#"newSP\((\d+(?:\.\d+)?|"(.*?)")\)"#).ok();

        for command in sub_html.split(';') {
            if let Some(caps) = create.as_ref().and_then(|r| r.captures(command)) {
                let start: i64 = caps[1].parse().unwrap_or(0);
                let end: i64 = caps[2].parse().unwrap_or(0);
                for value in start..=end {
                    names.push(value.to_string());
                }
                continue;
            }
            if let Some(caps) = special.as_ref().and_then(|r| r.captures(command)) {
                // Quoted names arrive with their quotes; drop them.
                let raw = caps
                    .get(2)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_else(|| caps[1].to_string());
                names.push(raw.trim_matches('"').to_string());
                specials += 1;
            }
        }
    }

    while names.len() < api_count {
        names.push((names.len() + 1 - specials).to_string());
    }

    // The id is the 1-based position, which is what the API is keyed by.
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| (name, index + 1))
        .collect()
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn text_of(element: scraper::ElementRef<'_>) -> String {
    element.text().collect::<String>().trim().to_string()
}

/// Stores urls without the domain, like upstream's `setUrlWithoutDomain`.
fn relative(url: &str) -> String {
    match url.strip_prefix(BASE) {
        Some(rest) if rest.starts_with('/') => rest.to_string(),
        _ => url.to_string(),
    }
}

fn join_path(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then(|| value.to_string())
    })
}

fn percent_decode(value: &str) -> String {
    urlencoding::decode(value)
        .map(|v| v.into_owned())
        .unwrap_or_else(|_| value.to_string())
}

/// Encodes a path segment, keeping it readable where it is already safe.
fn encode_path(value: &str) -> String {
    urlencoding::encode(value).into_owned()
}

/// Strips the "Scans" wording and brackets from a panel title.
fn clean_scanlator(panel_title: &str) -> String {
    let cleaned = regex::Regex::new(r"(Scans|\(|\))")
        .map(|r| r.replace_all(panel_title, "").into_owned())
        .unwrap_or_else(|_| panel_title.to_string());
    cleaned.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full chain against the live site: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_round_trip() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let http = Arc::new(HttpClient::new().unwrap());
            let source = AnimeSama::new(http.clone());

            let popular = source.popular(1).await.expect("popular failed");
            assert!(!popular.mangas.is_empty(), "catalogue returned nothing");
            println!(
                "popular: {} entries, first = {} ({})",
                popular.mangas.len(),
                popular.mangas[0].title,
                popular.mangas[0].url
            );
            assert!(popular.mangas[0].thumbnail_url.is_some(), "no cover url");
            assert!(
                !source.genres.read().is_empty(),
                "genre filters not picked up"
            );

            let details = source
                .details(&popular.mangas[0])
                .await
                .expect("details failed");
            assert!(!details.title.is_empty());
            println!(
                "details: {} | author={:?} | status={:?} | genres={}",
                details.title,
                details.author,
                details.status,
                details.genre.as_ref().map(|g| g.len()).unwrap_or(0)
            );

            // Not every entry has readable scans; walk until one does.
            let mut found = None;
            for candidate in popular.mangas.iter().take(6) {
                let chapters = source.chapters(candidate).await.unwrap_or_default();
                println!("{} -> {} chapter(s)", candidate.title, chapters.len());
                if !chapters.is_empty() {
                    found = Some((candidate.clone(), chapters));
                    break;
                }
            }
            let (manga, chapters) = found.expect("no entry produced chapters");
            println!("first chapter: {} ({})", chapters[0].name, chapters[0].url);
            assert!(chapters[0].name.starts_with("Chapitre "));

            let pages = source
                .pages(&manga, chapters.last().unwrap())
                .await
                .expect("page list failed");
            assert!(!pages.is_empty(), "no pages");
            println!("{} pages, first = {}", pages.len(), pages[0].image_url);

            // The image must really be fetchable with the referer header.
            let bytes = http
                .get_bytes(&pages[0].image_url, &pages[0].headers)
                .await
                .expect("image download failed");
            let decoded = image::load_from_memory(&bytes).expect("not an image");
            println!("page 1 decoded: {}x{}", decoded.width(), decoded.height());
            assert!(decoded.width() > 100);
        });
    }

    /// The series that exposed the trailing-space bug: it must come back with
    /// its full chapter list, not just the ones the page's script names.
    #[test]
    #[ignore]
    fn live_full_chapter_list() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let http = Arc::new(HttpClient::new().unwrap());
            let source = AnimeSama::new(http.clone());

            let manga = SManga::new(
                "/catalogue/i-obtained-a-mythic-item",
                "I Obtained a Mythic Item",
            );
            let chapters = source.chapters(&manga).await.expect("chapters failed");
            println!(
                "{} chapters, newest = {}, oldest = {}",
                chapters.len(),
                chapters.first().unwrap().name,
                chapters.last().unwrap().name
            );

            // The page's script alone would only yield 109 entries.
            assert!(
                chapters.len() > 150,
                "expected the full list, got {}",
                chapters.len()
            );
            assert_eq!(chapters.first().unwrap().name, "Chapitre 186");

            // And the newest chapter's pages must actually resolve.
            let pages = source
                .pages(&manga, chapters.first().unwrap())
                .await
                .expect("pages failed");
            assert!(!pages.is_empty());
            let bytes = http
                .get_bytes(&pages[0].image_url, &pages[0].headers)
                .await
                .expect("image download failed");
            assert!(image::load_from_memory(&bytes).is_ok());
            println!("newest chapter has {} readable pages", pages.len());
        });
    }

    #[test]
    fn scan_panels_are_read_from_the_inline_script() {
        // The page carries the template call first, then the real entries.
        let html = r#"<html><body><script>
            panneauScan("nom", "url");
            panneauScan("Scans (Team Alpha)", "scan/vf/");
            panneauScan("Scans VA", "scan/va/");
        </script></body></html>"#;

        let panels = parse_scan_panels(html);
        assert_eq!(panels.len(), 2, "the template call must be dropped");
        assert_eq!(panels[0], ("Scans (Team Alpha)".into(), "scan/vf/".into()));
        // The English release is filtered by the caller.
        assert!(panels[1].1.contains("va"));
    }

    #[test]
    fn scan_panels_survive_a_missing_template_call() {
        // Without the positional skip, a page that drops the template line
        // still yields every entry.
        let html = r#"<html><body><script>
            panneauScan("Scans (Team Beta)", "scan/vf/");
        </script></body></html>"#;
        let panels = parse_scan_panels(html);
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].1, "scan/vf/");
    }

    #[test]
    fn scanlator_names_lose_their_decoration() {
        assert_eq!(clean_scanlator("Scans (Team Alpha)"), "Team Alpha");
        assert_eq!(clean_scanlator("Scans"), "");
    }

    #[test]
    fn chapter_names_follow_the_reader_script() {
        let html = r#"resetListe(); creerListe(1, 3); newSP("Hors-série"); newSP(7.5);"#;
        let names = build_chapter_names(html, 0);
        let labels: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(labels, ["1", "2", "3", "Hors-série", "7.5"]);
        // Ids are the 1-based position, which is how the API keys its counts.
        assert_eq!(names[0].1, 1);
        assert_eq!(names[4].1, 5);
    }

    #[test]
    fn chapters_known_only_to_the_api_are_numbered() {
        // Two specials then five entries in total: the tail is numbered from 1.
        let html = r#"resetListe(); newSP("A"); newSP("B");"#;
        let names = build_chapter_names(html, 5);
        let labels: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(labels, ["A", "B", "1", "2", "3"]);
    }

    #[test]
    fn a_page_without_a_script_falls_back_to_the_api_count() {
        let names = build_chapter_names("<html></html>", 3);
        let labels: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(labels, ["1", "2", "3"]);
    }

    #[test]
    fn details_are_read_from_the_page() {
        let html = r#"<html><body>
            <div class="my-2"><h1>Ma Série</h1></div>
            <img id="coverOeuvre" src="/img/cover.jpg">
            <p id="synopsisText">Un synopsis.</p>
            <span class="genre-pill">Action</span><span class="genre-pill">Drame</span>
            <span class="info-lbl">Créateur</span><span class="info-val">Quelqu'un</span>
            <span class="info-lbl">État</span><span class="info-val">En cours</span>
        </body></html>"#;

        let manga = parse_details(html, "https://anime-sama.to/catalogue/ma-serie/");
        assert_eq!(manga.title, "Ma Série");
        assert_eq!(manga.url, "/catalogue/ma-serie/");
        assert_eq!(manga.description.as_deref(), Some("Un synopsis."));
        assert_eq!(manga.author.as_deref(), Some("Quelqu'un"));
        assert_eq!(manga.status, MangaStatus::Ongoing);
        assert_eq!(
            manga.thumbnail_url.as_deref(),
            Some("https://anime-sama.to/img/cover.jpg")
        );
        assert_eq!(manga.genre.unwrap(), vec!["Action", "Drame"]);
    }

    #[test]
    fn catalogue_cards_and_paging_are_read() {
        let html = r#"<html><body>
            <div id="list_catalog">
              <div><a href="/catalogue/un/"><h2 class="card-title">Un</h2></a><img src="/a.jpg"></div>
              <div><a href="/catalogue/deux/"><h2 class="card-title">Deux</h2></a><img src="/b.jpg"></div>
            </div>
            <div id="list_pagination"><a class="bg-sky-900">1</a><a>2</a></div>
        </body></html>"#;

        let (page, _) = parse_catalogue(html);
        assert_eq!(page.mangas.len(), 2);
        assert_eq!(page.mangas[0].title, "Un");
        assert_eq!(page.mangas[0].url, "/catalogue/un/");
        assert!(
            page.has_next_page,
            "a following page link means more results"
        );
    }

    #[test]
    fn the_last_catalogue_page_has_no_successor() {
        let html = r#"<html><body><div id="list_catalog"></div>
            <div id="list_pagination"><a>1</a><a class="bg-sky-900">2</a></div></body></html>"#;
        let (page, _) = parse_catalogue(html);
        assert!(!page.has_next_page);
    }

    #[test]
    fn latest_entries_drop_the_reader_suffix() {
        let html = r#"<html><body><div id="containerAjoutsScans">
            <div><a href="/catalogue/oeuvre/scan/vf/"><h2 class="card-title">Oeuvre</h2></a>
            <img src="/c.jpg"></div></div></body></html>"#;
        let page = parse_latest(html);
        assert_eq!(page.mangas.len(), 1);
        assert_eq!(page.mangas[0].url, "/catalogue/oeuvre/");
    }

    #[test]
    fn genres_are_collected_from_the_catalogue() {
        let html = r#"<html><body><div id="list_genres"><div id="genreList">
            <label><input name="genre[]" value="action"><span>Action</span></label>
            <label><input name="genre[]" value="drame"><span>Drame</span></label>
        </div></div></body></html>"#;
        let (_, genres) = parse_catalogue(html);
        assert_eq!(
            genres,
            vec![
                ("Action".to_string(), "action".to_string()),
                ("Drame".to_string(), "drame".to_string()),
            ]
        );
    }

    #[test]
    fn the_work_title_ignores_alternative_titles() {
        let html = r#"<html><body><h1 id="titreOeuvre">Vraie Série<span>Autre titre</span></h1></body></html>"#;
        assert_eq!(parse_work_title(html), "Vraie Série");
    }

    /// Regression: several works are stored with a trailing space and the
    /// chapter API matches the title byte for byte. Trimming it made the API
    /// answer "not found", so the list fell back to whatever the page's script
    /// happened to name — 109 chapters instead of 188 for one series.
    #[test]
    fn the_work_title_keeps_its_trailing_space() {
        let html =
            r#"<html><body><h1 id="titreOeuvre">I Obtained a Mythic Item </h1></body></html>"#;
        assert_eq!(parse_work_title(html), "I Obtained a Mythic Item ");
        assert_eq!(
            urlencoding::encode(&parse_work_title(html)),
            "I%20Obtained%20a%20Mythic%20Item%20"
        );
    }

    /// `finirListe(debut)` is not replayed directly; the trailing pad has to
    /// reproduce it exactly, using the API's chapter count.
    #[test]
    fn the_tail_reproduces_finir_liste() {
        // The real script for "I Obtained a Mythic Item".
        let html = r#"resetListe();newSP(0);
                      creerListe(1, 107);newSP("Bonus");
                      finirListe(108);"#;
        let names = build_chapter_names(html, 188);

        assert_eq!(names.len(), 188);
        assert_eq!(names[0].0, "0", "the first special leads");
        assert_eq!(names[1].0, "1");
        assert_eq!(names[107].0, "107");
        assert_eq!(names[108].0, "Bonus");
        // finirListe(108) fills 108 up to (count - specials) = 186.
        assert_eq!(names[109].0, "108", "the tail must resume at 108");
        assert_eq!(names.last().unwrap().0, "186");
        // Ids stay the 1-based position the API is keyed by.
        assert_eq!(names.last().unwrap().1, 188);
    }

    #[test]
    fn query_parameters_are_extracted() {
        let url = "/s2/scans/get_nb_chap_et_img.php?oeuvre=Ma%20S%C3%A9rie&id=4&title=x";
        assert_eq!(query_param(url, "id").as_deref(), Some("4"));
        assert_eq!(
            percent_decode(&query_param(url, "oeuvre").unwrap()),
            "Ma Série"
        );
        assert_eq!(query_param(url, "absent"), None);
    }
}
