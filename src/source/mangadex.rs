//! MangaDex, through its public JSON API (api.mangadex.org).
//!
//! Registered once per translated language, the way Mihon ships one MangaDex
//! extension entry per language.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::Value;

use super::{
    Filter, FilterKind, FilterList, MangasPage, Page, SChapter, SManga, Source, for_each_filter,
    generate_id,
};
use crate::model::{MangaStatus, TriState};
use crate::net::HttpClient;

const API: &str = "https://api.mangadex.org";
const COVERS: &str = "https://uploads.mangadex.org/covers";
const WEB: &str = "https://mangadex.org";
const PAGE_SIZE: u32 = 24;
/// Bumping this changes every source id, exactly like upstream's versionId.
const VERSION: u32 = 1;

/// Languages the app registers a MangaDex entry for.
pub const SUPPORTED_LANGS: &[&str] = &["en", "fr", "es", "es-la", "pt-br", "de", "it", "ru", "ja"];

pub struct MangaDex {
    id: i64,
    name: String,
    lang: &'static str,
    http: Arc<HttpClient>,
}

impl MangaDex {
    pub fn new(http: Arc<HttpClient>, lang: &'static str) -> Self {
        // MangaDex asks for at most 5 requests per second per client.
        http.set_rate_limit("api.mangadex.org", 5, Duration::from_secs(1));
        let name = "MangaDex".to_string();
        Self {
            id: generate_id(&name, lang, VERSION),
            name,
            lang,
            http,
        }
    }

    fn common_query(&self) -> String {
        let mut q = String::new();
        q.push_str("&includes[]=cover_art&includes[]=author&includes[]=artist");
        q.push_str(&format!("&availableTranslatedLanguage[]={}", self.lang));
        q.push_str("&hasAvailableChapters=true");
        q
    }

    /// Turns the browse filter list into MangaDex query parameters.
    fn filters_to_query(&self, filters: &FilterList) -> String {
        let mut query = String::new();
        let mut ordering: Option<(String, bool)> = None;

        for_each_filter(filters, &mut |filter| match &filter.kind {
            FilterKind::Sort {
                values,
                index,
                ascending,
                ..
            } => {
                if let Some(value) = values.get(*index) {
                    ordering = Some((value.clone(), *ascending));
                }
            }
            FilterKind::CheckBox { checked, value } if *checked => {
                query.push_str(value);
            }
            FilterKind::Tri { state, value } => match state {
                TriState::EnabledIs => query.push_str(&format!("&includedTags[]={value}")),
                TriState::EnabledNot => query.push_str(&format!("&excludedTags[]={value}")),
                TriState::Disabled => {}
            },
            FilterKind::Text { value } if !value.trim().is_empty() => {
                query.push_str(&format!(
                    "&authorOrArtist={}",
                    urlencoding::encode(value.trim())
                ));
            }
            _ => {}
        });

        let (order_key, ascending) = ordering.unwrap_or_else(|| ("followedCount".into(), false));
        let direction = if ascending { "asc" } else { "desc" };
        query.push_str(&format!("&order[{order_key}]={direction}"));

        if !query.contains("contentRating[]") {
            query.push_str("&contentRating[]=safe&contentRating[]=suggestive");
        }
        query
    }

    async fn list(&self, page: u32, extra: &str) -> Result<MangasPage> {
        let offset = (page.saturating_sub(1)) * PAGE_SIZE;
        let url = format!(
            "{API}/manga?limit={PAGE_SIZE}&offset={offset}{}{extra}",
            self.common_query()
        );
        let json = self.http.get_json(&url, &[]).await?;
        parse_manga_list(&json)
    }
}

#[async_trait]
impl Source for MangaDex {
    fn id(&self) -> i64 {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn lang(&self) -> &str {
        self.lang
    }

    fn base_url(&self) -> &str {
        WEB
    }

    fn filters(&self) -> FilterList {
        vec![
            Filter::sort(
                "Sort",
                &[
                    ("Popularity", "followedCount"),
                    ("Latest upload", "latestUploadedChapter"),
                    ("Relevance", "relevance"),
                    ("Title", "title"),
                    ("Year", "year"),
                    ("Rating", "rating"),
                    ("Created at", "createdAt"),
                ],
                0,
                false,
            ),
            Filter::text("Author or artist"),
            Filter::separator(),
            Filter::group(
                "Content rating",
                vec![
                    Filter::checkbox("Safe", "&contentRating[]=safe"),
                    Filter::checkbox("Suggestive", "&contentRating[]=suggestive"),
                    Filter::checkbox("Erotica", "&contentRating[]=erotica"),
                    Filter::checkbox("Pornographic", "&contentRating[]=pornographic"),
                ],
            ),
            Filter::group(
                "Publication status",
                vec![
                    Filter::checkbox("Ongoing", "&status[]=ongoing"),
                    Filter::checkbox("Completed", "&status[]=completed"),
                    Filter::checkbox("Hiatus", "&status[]=hiatus"),
                    Filter::checkbox("Cancelled", "&status[]=cancelled"),
                ],
            ),
            Filter::group(
                "Demographic",
                vec![
                    Filter::checkbox("Shounen", "&publicationDemographic[]=shounen"),
                    Filter::checkbox("Shoujo", "&publicationDemographic[]=shoujo"),
                    Filter::checkbox("Seinen", "&publicationDemographic[]=seinen"),
                    Filter::checkbox("Josei", "&publicationDemographic[]=josei"),
                ],
            ),
            Filter::group(
                "Original language",
                vec![
                    Filter::checkbox("Japanese", "&originalLanguage[]=ja"),
                    Filter::checkbox("Korean", "&originalLanguage[]=ko"),
                    Filter::checkbox("Chinese", "&originalLanguage[]=zh"),
                ],
            ),
            Filter::separator(),
            Filter::header("Tags"),
            Filter::group(
                "Genres",
                TAGS.iter()
                    .map(|(name, id)| Filter::tri(name, id))
                    .collect(),
            ),
        ]
    }

    async fn popular(&self, page: u32) -> Result<MangasPage> {
        self.list(
            page,
            "&order[followedCount]=desc&contentRating[]=safe&contentRating[]=suggestive",
        )
        .await
    }

    async fn latest(&self, page: u32) -> Result<MangasPage> {
        self.list(
            page,
            "&order[latestUploadedChapter]=desc&contentRating[]=safe&contentRating[]=suggestive",
        )
        .await
    }

    async fn search(&self, page: u32, query: &str, filters: &FilterList) -> Result<MangasPage> {
        let mut extra = self.filters_to_query(filters);
        if !query.trim().is_empty() {
            extra.push_str(&format!("&title={}", urlencoding::encode(query.trim())));
        }
        self.list(page, &extra).await
    }

    async fn details(&self, manga: &SManga) -> Result<SManga> {
        let uuid = uuid_of(&manga.url)?;
        let url =
            format!("{API}/manga/{uuid}?includes[]=cover_art&includes[]=author&includes[]=artist");
        let json = self.http.get_json(&url, &[]).await?;
        let data = json
            .get("data")
            .context("MangaDex response has no `data`")?;
        let mut parsed = parse_manga(data).context("could not parse the manga record")?;
        parsed.initialized = true;
        Ok(parsed)
    }

    async fn chapters(&self, manga: &SManga) -> Result<Vec<SChapter>> {
        let uuid = uuid_of(&manga.url)?;
        let mut chapters = Vec::new();
        let mut offset = 0u32;

        // The feed endpoint caps out at 500 rows, so walk it until exhausted.
        loop {
            let url = format!(
                "{API}/manga/{uuid}/feed?limit=500&offset={offset}\
                 &translatedLanguage[]={lang}&includes[]=scanlation_group\
                 &order[volume]=desc&order[chapter]=desc\
                 &contentRating[]=safe&contentRating[]=suggestive\
                 &contentRating[]=erotica&contentRating[]=pornographic",
                lang = self.lang
            );
            let json = self.http.get_json(&url, &[]).await?;
            let data = json
                .get("data")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let received = data.len() as u32;

            for entry in &data {
                // Chapters hosted elsewhere cannot be read in-app; skip them.
                if entry
                    .pointer("/attributes/externalUrl")
                    .and_then(Value::as_str)
                    .is_some()
                {
                    continue;
                }
                if let Some(chapter) = parse_chapter(entry) {
                    chapters.push(chapter);
                }
            }

            let total = json.get("total").and_then(Value::as_u64).unwrap_or(0) as u32;
            offset += received.max(1);
            if received == 0 || offset >= total {
                break;
            }
        }

        // Newest first, matching how the source presents them.
        chapters.sort_by(|a, b| b.chapter_number.total_cmp(&a.chapter_number));
        Ok(chapters)
    }

    async fn pages(&self, _manga: &SManga, chapter: &SChapter) -> Result<Vec<Page>> {
        let uuid = uuid_of(&chapter.url)?;
        let url = format!("{API}/at-home/server/{uuid}");
        let json = self.http.get_json(&url, &[]).await?;

        let base = json
            .get("baseUrl")
            .and_then(Value::as_str)
            .context("at-home response has no baseUrl")?;
        let hash = json
            .pointer("/chapter/hash")
            .and_then(Value::as_str)
            .context("at-home response has no chapter hash")?;
        let files = json
            .pointer("/chapter/data")
            .and_then(Value::as_array)
            .context("at-home response has no page list")?;

        if files.is_empty() {
            bail!("MangaDex returned no pages for this chapter");
        }

        // The compressed variant is a genuine fallback: individual files are
        // sometimes missing from a network node while the other list serves.
        let saver: Vec<&str> = json
            .pointer("/chapter/dataSaver")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();

        Ok(files
            .iter()
            .filter_map(Value::as_str)
            .enumerate()
            .map(|(index, file)| Page {
                index,
                image_url: format!("{base}/data/{hash}/{file}"),
                fallbacks: saver
                    .get(index)
                    .map(|alt| vec![format!("{base}/data-saver/{hash}/{alt}")])
                    .unwrap_or_default(),
                headers: Vec::new(),
            })
            .collect())
    }

    fn web_url(&self, manga: &SManga) -> String {
        match uuid_of(&manga.url) {
            Ok(uuid) => format!("{WEB}/title/{uuid}"),
            Err(_) => WEB.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Manga and chapter urls are stored as `/manga/<uuid>` and `/chapter/<uuid>`.
fn uuid_of(url: &str) -> Result<&str> {
    url.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .context("malformed MangaDex url")
}

fn parse_manga_list(json: &Value) -> Result<MangasPage> {
    let data = json
        .get("data")
        .and_then(Value::as_array)
        .context("MangaDex response has no `data` array")?;

    let mangas: Vec<SManga> = data.iter().filter_map(parse_manga).collect();

    let limit = json.get("limit").and_then(Value::as_u64).unwrap_or(0);
    let offset = json.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let total = json.get("total").and_then(Value::as_u64).unwrap_or(0);

    Ok(MangasPage {
        mangas,
        has_next_page: offset + limit < total,
    })
}

fn parse_manga(entry: &Value) -> Option<SManga> {
    let id = entry.get("id").and_then(Value::as_str)?;
    let attributes = entry.get("attributes")?;

    let title = localized(attributes.get("title"))
        .or_else(|| {
            attributes
                .get("altTitles")
                .and_then(Value::as_array)
                .and_then(|list| list.iter().find_map(|t| localized(Some(t))))
        })
        .unwrap_or_else(|| "Untitled".to_string());

    let description = localized(attributes.get("description"));

    let genre: Vec<String> = attributes
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(|t| localized(t.pointer("/attributes/name")))
                .collect()
        })
        .unwrap_or_default();

    let status = match attributes.get("status").and_then(Value::as_str) {
        Some("ongoing") => MangaStatus::Ongoing,
        Some("completed") => MangaStatus::Completed,
        Some("hiatus") => MangaStatus::OnHiatus,
        Some("cancelled") => MangaStatus::Cancelled,
        _ => MangaStatus::Unknown,
    };

    let relationships = entry
        .get("relationships")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let thumbnail_url = relationships
        .iter()
        .find(|r| r.get("type").and_then(Value::as_str) == Some("cover_art"))
        .and_then(|r| r.pointer("/attributes/fileName").and_then(Value::as_str))
        .map(|file| format!("{COVERS}/{id}/{file}.512.jpg"));

    let author = relationship_name(&relationships, "author");
    let artist = relationship_name(&relationships, "artist");

    Some(SManga {
        url: format!("/manga/{id}"),
        title,
        artist,
        author,
        description,
        genre: (!genre.is_empty()).then_some(genre),
        status,
        thumbnail_url,
        update_strategy: crate::model::UpdateStrategy::AlwaysUpdate,
        initialized: false,
    })
}

fn relationship_name(relationships: &[Value], kind: &str) -> Option<String> {
    relationships
        .iter()
        .find(|r| r.get("type").and_then(Value::as_str) == Some(kind))
        .and_then(|r| r.pointer("/attributes/name").and_then(Value::as_str))
        .map(str::to_string)
}

fn parse_chapter(entry: &Value) -> Option<SChapter> {
    let id = entry.get("id").and_then(Value::as_str)?;
    let attributes = entry.get("attributes")?;

    let volume = attributes.get("volume").and_then(Value::as_str);
    let number = attributes.get("chapter").and_then(Value::as_str);
    let title = attributes
        .get("title")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty());

    // "Vol.2 Ch.14 - The Title", omitting whichever parts are missing.
    let mut name = String::new();
    if let Some(volume) = volume {
        name.push_str(&format!("Vol.{volume} "));
    }
    match number {
        Some(n) => name.push_str(&format!("Ch.{n}")),
        None => name.push_str("Oneshot"),
    }
    if let Some(title) = title {
        name.push_str(&format!(" - {title}"));
    }

    let scanlator = entry
        .get("relationships")
        .and_then(Value::as_array)
        .and_then(|rels| relationship_name(rels, "scanlation_group"));

    let date_upload = attributes
        .get("publishAt")
        .and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0);

    Some(SChapter {
        url: format!("/chapter/{id}"),
        name: name.trim().to_string(),
        date_upload,
        chapter_number: number.and_then(|n| n.parse::<f64>().ok()).unwrap_or(-1.0),
        scanlator,
    })
}

/// MangaDex returns localised maps like `{"en": "...", "ja": "..."}`;
/// take English when present, otherwise whatever is first.
fn localized(value: Option<&Value>) -> Option<String> {
    let map = value?.as_object()?;
    map.get("en")
        .and_then(Value::as_str)
        .or_else(|| map.values().find_map(Value::as_str))
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// A curated slice of MangaDex's tag list, as include/exclude filters.
const TAGS: &[(&str, &str)] = &[
    ("Action", "391b0423-d847-456f-aff0-8b0cfc03066b"),
    ("Adventure", "87cc87cd-a395-47af-b27a-93258283bbc6"),
    ("Comedy", "4d32cc48-9f00-4cca-9b5a-a839f0764984"),
    ("Crime", "5ca48985-9a9d-4bd8-be29-80dc0303db72"),
    ("Drama", "b9af3a63-f058-46de-a9a0-e0c13906197a"),
    ("Fantasy", "cdc58593-87dd-415e-bbc0-2ec27bf404cc"),
    ("Historical", "33771934-028e-4cb3-8744-691e866a923e"),
    ("Horror", "cdad7e68-1419-41dd-bdce-27753074a640"),
    ("Isekai", "ace04997-f6bd-436e-b261-779182193d3d"),
    ("Mecha", "50880a9d-5440-4732-9afb-8f457127e836"),
    ("Medical", "c8cbe35b-1b2b-4a3f-9c37-db84c4514856"),
    ("Mystery", "ee968100-4191-4968-93d3-f82d72be7e46"),
    ("Philosophical", "b1e97889-25b4-4258-b28b-cd7f4d28ea9b"),
    ("Psychological", "3b60b75c-a2d7-4860-ab56-05f391bb889c"),
    ("Romance", "423e2eae-a7a2-4a8b-ac03-a8351462d71d"),
    ("Sci-Fi", "256c8bd9-4904-4360-bf4f-508a76d67183"),
    ("Slice of Life", "e5301a23-ebd9-49dd-a0cb-2add944c7fe9"),
    ("Sports", "69964a64-2f90-4d33-beeb-f3ed2875eb4c"),
    ("Superhero", "7064a261-a137-4d3a-8848-2d385de3a99c"),
    ("Thriller", "07251805-a27e-4d59-b488-f0bfbec15168"),
    ("Tragedy", "f8f62932-27da-4fe4-8ee1-6779a8c5edba"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_are_extracted_from_stored_urls() {
        assert_eq!(uuid_of("/manga/abc-123").unwrap(), "abc-123");
        assert!(uuid_of("/manga/").is_err());
    }

    #[test]
    fn localized_prefers_english() {
        let value = serde_json::json!({"ja": "ワンピース", "en": "One Piece"});
        assert_eq!(localized(Some(&value)).unwrap(), "One Piece");
        let value = serde_json::json!({"ja": "ワンピース"});
        assert_eq!(localized(Some(&value)).unwrap(), "ワンピース");
    }

    /// End-to-end check against the live API. Excluded from the default run
    /// because it needs network access: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_api_round_trip() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let http = Arc::new(HttpClient::new().unwrap());
            let source = MangaDex::new(http, "en");

            let popular = source.popular(1).await.expect("popular listing failed");
            assert!(!popular.mangas.is_empty(), "no popular entries returned");
            let first = &popular.mangas[0];
            assert!(!first.title.is_empty());
            assert!(first.thumbnail_url.is_some(), "cover url missing");
            println!("popular[0] = {} ({})", first.title, first.url);

            let details = source.details(first).await.expect("details failed");
            assert!(details.initialized);
            assert_eq!(details.url, first.url);

            // Licensed titles expose only externally-hosted chapters, which are
            // deliberately skipped, so walk the listing until one is readable.
            let mut readable = None;
            for candidate in popular.mangas.iter().take(6) {
                let chapters = source
                    .chapters(candidate)
                    .await
                    .expect("chapter list failed");
                println!(
                    "{} -> {} readable chapter(s)",
                    candidate.title,
                    chapters.len()
                );
                if !chapters.is_empty() {
                    readable = Some((candidate.clone(), chapters));
                    break;
                }
            }
            let (manga, chapters) =
                readable.expect("none of the popular entries had readable chapters");

            // Chapters come back newest-first.
            assert!(chapters[0].chapter_number >= chapters[chapters.len() - 1].chapter_number);
            assert!(chapters.iter().all(|c| c.url.starts_with("/chapter/")));

            let last = chapters.last().unwrap();
            let pages = source.pages(&manga, last).await.expect("page list failed");
            assert!(!pages.is_empty(), "no pages returned");
            assert!(pages[0].image_url.starts_with("http"));
            println!("{} pages, first = {}", pages.len(), pages[0].image_url);

            // The page must actually be fetchable, not merely a plausible url.
            let http2 = HttpClient::new().unwrap();
            let bytes = http2
                .get_bytes(&pages[0].image_url, &[])
                .await
                .expect("page image download failed");
            let decoded = image::load_from_memory(&bytes).expect("page image is not decodable");
            println!("page 1 decoded: {}x{}", decoded.width(), decoded.height());
            assert!(decoded.width() > 100 && decoded.height() > 100);

            let search = source
                .search(1, "one piece", &source.filters())
                .await
                .expect("search failed");
            assert!(!search.mangas.is_empty(), "search returned nothing");
        });
    }

    #[test]
    fn chapter_names_are_composed() {
        let entry = serde_json::json!({
            "id": "cid",
            "attributes": {"volume": "2", "chapter": "14", "title": "Sunrise",
                           "publishAt": "2020-01-02T03:04:05+00:00"},
            "relationships": []
        });
        let chapter = parse_chapter(&entry).unwrap();
        assert_eq!(chapter.name, "Vol.2 Ch.14 - Sunrise");
        assert_eq!(chapter.chapter_number, 14.0);
        assert_eq!(chapter.url, "/chapter/cid");
    }
}
