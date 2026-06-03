use async_openai::{
    Client as OpenAIClient,
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        CreateChatCompletionRequestArgs,
    },
};
use chrono::{Datelike, Local, Weekday};
use dotenv::dotenv;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    env, fs,
    io::{self, Write},
    process,
};
use tokio::time::{Duration, sleep};

const PROCESSED_URLS_FILE: &str = "processed_urls.txt";
const ARTICLE_TEXT_LIMIT: usize = 5_000;
const MAX_ARTICLES_PER_SESSION: usize = 8;
const FEED_DELAY_SECONDS: u64 = 2;
const OPENAI_DELAY_SECONDS: u64 = 5;
const CERTIFICATION_WEEKDAY: Weekday = Weekday::Mon;
const OPENAI_SYSTEM_PROMPT: &str = r#"Anda adalah analis teknologi. Ringkaslah artikel-artikel berikut ke dalam format JSON dengan tepat 5 kunci: "agentic_ai" (arsitektur/tren agen AI), "architecture_ai" (arsitektur AI baru), "programming" (bug dan update versi), "tech_update" (update teknologi umum: AI, cloud, dan programming), dan "certifications" (ujian, pelatihan, voucher, atau program sertifikasi). Setiap kategori harus berisi array objek dengan struktur {"title": "...", "summary": "...", "url": "..."}. Untuk agentic_ai, architecture_ai, programming, dan tech_update, berikan maksimal 2 artikel jika tersedia. Untuk certifications, hanya isi jika artikel membahas ujian, pelatihan, voucher, cohort, program belajar, atau sertifikasi yang relevan dengan region ASEAN/Indonesia; jika tidak relevan, gunakan array kosong. Contoh format: {"agentic_ai":[{"title":"...","summary":"...","url":"..."}],"architecture_ai":[{"title":"...","summary":"...","url":"..."}],"programming":[{"title":"...","summary":"...","url":"..."}],"tech_update":[{"title":"...","summary":"...","url":"..."}],"certifications":[{"title":"...","summary":"...","url":"..."}]}. Gunakan URL artikel asli yang diberikan. Jangan sertakan teks lain selain JSON tersebut."#;

#[derive(Debug, Clone)]
struct Article {
    source_feed: String,
    title: String,
    url: String,
    summary: String,
}

#[derive(Debug, Deserialize)]
struct CategorizedSummary {
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    agentic_ai: Vec<ArticleInfo>,
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    architecture_ai: Vec<ArticleInfo>,
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    programming: Vec<ArticleInfo>,
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    tech_update: Vec<ArticleInfo>,
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    certifications: Vec<ArticleInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct ArticleInfo {
    #[serde(default, deserialize_with = "deserialize_null_string_as_default")]
    title: String,
    #[serde(default, deserialize_with = "deserialize_null_string_as_default")]
    summary: String,
    #[serde(default, deserialize_with = "deserialize_null_string_as_default")]
    url: String,
}

impl CategorizedSummary {
    fn empty() -> Self {
        Self {
            agentic_ai: Vec::new(),
            architecture_ai: Vec::new(),
            programming: Vec::new(),
            tech_update: Vec::new(),
            certifications: Vec::new(),
        }
    }

    fn merge(&mut self, other: CategorizedSummary) {
        self.agentic_ai.extend(other.agentic_ai);
        self.architecture_ai.extend(other.architecture_ai);
        self.programming.extend(other.programming);
        self.tech_update.extend(other.tech_update);
        self.certifications.extend(other.certifications);
    }
}

fn deserialize_null_as_default<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

fn deserialize_null_string_as_default<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

#[tokio::main]
async fn main() {
    dotenv().ok();

    if let Err(error) = run().await {
        eprintln!("Error: {error}");
        process::exit(0);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let openai_api_key = read_env("OPENAI_API_KEY")?;
    let discord_webhook_url = read_env("DISCORD_WEBHOOK_URL")?;
    let processed_urls = read_processed_urls(PROCESSED_URLS_FILE)?;
    let http_client = HttpClient::new();

    let mut new_articles = Vec::new();

    let mut feeds = rss_feeds();

    if should_check_certifications() {
        println!("Hari sertifikasi aktif. Feed sertifikasi akan diproses.");
        feeds.extend(certification_rss_feeds());
    } else {
        println!(
            "Bukan hari sertifikasi ({:?}). Feed sertifikasi dilewati.",
            CERTIFICATION_WEEKDAY
        );
    }

    for (index, feed_url) in feeds.iter().enumerate() {
        match fetch_rss_feed(&http_client, feed_url).await {
            Ok(feed_content) => {
                let articles = parse_feed_articles(feed_url, &feed_content);

                for article in articles {
                    if processed_urls.contains(&article.url) {
                        println!("Lewati artikel yang sudah diproses: {}", article.url);
                        continue;
                    }

                    new_articles.push(article);

                    if new_articles.len() >= MAX_ARTICLES_PER_SESSION {
                        println!(
                            "Batas {} artikel baru per sesi tercapai.",
                            MAX_ARTICLES_PER_SESSION
                        );
                        break;
                    }
                }
            }
            Err(error) => eprintln!("Feed dilewati karena gagal fetch: {feed_url} ({error})"),
        }

        if new_articles.len() >= MAX_ARTICLES_PER_SESSION {
            break;
        }

        if index + 1 < feeds.len() {
            sleep(Duration::from_secs(FEED_DELAY_SECONDS)).await;
        }
    }

    if new_articles.is_empty() {
        println!("Tidak ada artikel baru untuk diproses.");
        return Ok(());
    }

    let summary = summarize_articles_one_by_one(&openai_api_key, &new_articles).await?;
    let discord_embeds = format_discord_message(&summary);

    send_to_discord(&http_client, &discord_webhook_url, &discord_embeds).await?;
    append_processed_urls(PROCESSED_URLS_FILE, &new_articles)?;

    println!(
        "Berhasil mengirim ringkasan dan menyimpan {} URL artikel baru.",
        new_articles.len()
    );

    Ok(())
}

fn rss_feeds() -> Vec<String> {
    vec![
        "https://blog.langchain.dev/rss/".to_string(),
        "https://github.blog/feed".to_string(),
        "https://security.googleblog.com/feeds/posts/default".to_string(),
        "https://openai.com/blog/rss.xml".to_string(),
        "https://techcrunch.com/category/artificial-intelligence/feed".to_string(),
    ]
}

fn certification_rss_feeds() -> Vec<String> {
    vec![
        "https://aws.amazon.com/blogs/training-and-certification/feed/".to_string(),
        "https://cloud.google.com/blog/topics/training-certifications/rss.xml".to_string(),
        "https://www.dicoding.com/blog/feed/".to_string(),
    ]
}

fn should_check_certifications() -> bool {
    Local::now().weekday() == CERTIFICATION_WEEKDAY
}

fn read_env(key: &str) -> Result<String, Box<dyn std::error::Error>> {
    match env::var(key) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(format!("Variabel environment {key} belum diisi.").into()),
    }
}

fn read_processed_urls(path: &str) -> io::Result<HashSet<String>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(content
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(HashSet::new()),
        Err(error) => Err(error),
    }
}

fn append_processed_urls(path: &str, articles: &[Article]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    for article in articles {
        writeln!(file, "{}", article.url)?;
    }

    Ok(())
}

async fn fetch_rss_feed(
    http_client: &HttpClient,
    url: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let response = http_client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(format!("Status HTTP {}", response.status()).into());
    }

    Ok(response.text().await?)
}

fn parse_feed_articles(feed_url: &str, feed_content: &str) -> Vec<Article> {
    let mut articles = parse_rss_items(feed_url, feed_content);

    if articles.is_empty() {
        articles = parse_atom_entries(feed_url, feed_content);
    }

    articles
}

fn parse_rss_items(feed_url: &str, feed_content: &str) -> Vec<Article> {
    find_blocks(feed_content, "<item", "</item>")
        .into_iter()
        .filter_map(|item| {
            let title =
                extract_tag_text(&item, "title").unwrap_or_else(|| "Tanpa judul".to_string());
            let url = extract_tag_text(&item, "link")?;
            let summary = extract_tag_text(&item, "description")
                .or_else(|| extract_tag_text(&item, "content:encoded"))
                .unwrap_or_default();

            Some(Article {
                source_feed: feed_url.to_string(),
                title: clean_text(&title),
                url: clean_text(&url),
                summary: clean_text(&summary),
            })
        })
        .collect()
}

fn parse_atom_entries(feed_url: &str, feed_content: &str) -> Vec<Article> {
    find_blocks(feed_content, "<entry", "</entry>")
        .into_iter()
        .filter_map(|entry| {
            let title =
                extract_tag_text(&entry, "title").unwrap_or_else(|| "Tanpa judul".to_string());
            let url = extract_atom_link(&entry)?;
            let summary = extract_tag_text(&entry, "summary")
                .or_else(|| extract_tag_text(&entry, "content"))
                .unwrap_or_default();

            Some(Article {
                source_feed: feed_url.to_string(),
                title: clean_text(&title),
                url: clean_text(&url),
                summary: clean_text(&summary),
            })
        })
        .collect()
}

fn find_blocks(content: &str, start_marker: &str, end_marker: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut remaining = content;

    while let Some(start_index) = remaining.find(start_marker) {
        let after_start = &remaining[start_index..];

        if let Some(end_index) = after_start.find(end_marker) {
            let block_end = end_index + end_marker.len();
            blocks.push(after_start[..block_end].to_string());
            remaining = &after_start[block_end..];
        } else {
            break;
        }
    }

    blocks
}

fn extract_tag_text(content: &str, tag: &str) -> Option<String> {
    let start_marker = format!("<{tag}");
    let end_marker = format!("</{tag}>");
    let start_index = content.find(&start_marker)?;
    let after_start = &content[start_index..];
    let close_index = after_start.find('>')?;
    let text_start = close_index + 1;
    let text_end = after_start[text_start..].find(&end_marker)? + text_start;

    Some(after_start[text_start..text_end].to_string())
}

fn extract_atom_link(entry: &str) -> Option<String> {
    let mut remaining = entry;

    while let Some(link_index) = remaining.find("<link") {
        let after_link = &remaining[link_index..];
        let tag_end = after_link.find('>')?;
        let link_tag = &after_link[..tag_end];

        if let Some(href) = extract_attribute(link_tag, "href") {
            if link_tag.contains("rel=\"alternate\"")
                || link_tag.contains("rel='alternate'")
                || !link_tag.contains("rel=")
            {
                return Some(href);
            }
        }

        remaining = &after_link[tag_end..];
    }

    None
}

fn extract_attribute(tag: &str, attribute: &str) -> Option<String> {
    let double_quote_marker = format!("{attribute}=\"");
    let single_quote_marker = format!("{attribute}='");

    if let Some(start_index) = tag.find(&double_quote_marker) {
        let value_start = start_index + double_quote_marker.len();
        let value_end = tag[value_start..].find('"')? + value_start;
        return Some(tag[value_start..value_end].to_string());
    }

    if let Some(start_index) = tag.find(&single_quote_marker) {
        let value_start = start_index + single_quote_marker.len();
        let value_end = tag[value_start..].find('\'')? + value_start;
        return Some(tag[value_start..value_end].to_string());
    }

    None
}

fn clean_text(value: &str) -> String {
    value
        .replace("<![CDATA[", "")
        .replace("]]>", "")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

async fn summarize_articles_one_by_one(
    openai_api_key: &str,
    articles: &[Article],
) -> Result<CategorizedSummary, Box<dyn std::error::Error>> {
    let client = OpenAIClient::with_config(OpenAIConfig::new().with_api_key(openai_api_key));
    let mut combined_summary = CategorizedSummary::empty();

    for (index, article) in articles.iter().enumerate() {
        let summary = summarize_article(&client, article).await?;
        combined_summary.merge(summary);

        if index + 1 < articles.len() {
            sleep(Duration::from_secs(OPENAI_DELAY_SECONDS)).await;
        }
    }

    Ok(combined_summary)
}

async fn summarize_article(
    client: &OpenAIClient<OpenAIConfig>,
    article: &Article,
) -> Result<CategorizedSummary, Box<dyn std::error::Error>> {
    let prompt = build_article_prompt(article);

    let request = CreateChatCompletionRequestArgs::default()
        .model("gpt-4o")
        .messages([
            ChatCompletionRequestSystemMessageArgs::default()
                .content(OPENAI_SYSTEM_PROMPT)
                .build()?
                .into(),
            ChatCompletionRequestUserMessageArgs::default()
                .content(prompt)
                .build()?
                .into(),
        ])
        .build()?;

    let response = client.chat().create(request).await?;
    let raw_summary_json = response
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .ok_or("OpenAI tidak mengembalikan ringkasan.")?;

    println!("Raw JSON dari AI: {}", &raw_summary_json);

    let cleaned_json = strip_json_code_fence(&raw_summary_json);
    let summary: CategorizedSummary = serde_json::from_str(&cleaned_json)?;

    Ok(summary)
}

fn build_article_prompt(article: &Article) -> String {
    let truncated_summary = truncate_text(&article.summary, ARTICLE_TEXT_LIMIT);

    format!(
        "Artikel baru yang perlu dianalisis:\n\nJudul: {}\nURL: {}\nSumber RSS: {}\nCuplikan: {}\n",
        article.title, article.url, article.source_feed, truncated_summary
    )
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut truncated = chars.by_ref().take(max_chars).collect::<String>();

    if chars.next().is_some() {
        truncated.push_str("...");
    }

    truncated
}

fn format_discord_message(summary: &CategorizedSummary) -> Vec<Value> {
    let mut categories = vec![
        ("Agentic AI", &summary.agentic_ai),
        ("Cloud Architecture", &summary.architecture_ai),
        ("Programming", &summary.programming),
        ("Tech Updates", &summary.tech_update),
    ];

    if Local::now().weekday() == Weekday::Mon {
        categories.push(("Certifications", &summary.certifications));
    }

    categories
        .into_iter()
        .map(|(category_name, articles)| {
            json!({
                "title": format!("📰 LINE TECH NEWS | {category_name}"),
                "color": category_color(category_name),
                "description": format_embed_description(articles),
                "footer": {
                    "text": "Tech Radar • Morning Digest"
                }
            })
        })
        .collect()
}

fn category_color(category_name: &str) -> u32 {
    match category_name {
        "Agentic AI" => 46_714,
        "Cloud Architecture" => 39_423,
        "Programming" => 10_182_117,
        "Tech Updates" => 15_817_653,
        _ => 16_777_215,
    }
}

fn strip_json_code_fence(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
        .to_string()
}

fn format_embed_description(points: &[ArticleInfo]) -> String {
    if points.is_empty() {
        return "Tidak ada artikel baru yang relevan.".to_string();
    }

    points
        .iter()
        .take(2)
        .map(format_embed_article_item)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_embed_article_item(article: &ArticleInfo) -> String {
    let title = if article.title.trim().is_empty() {
        let summary_fallback = article.summary.trim();

        if summary_fallback.is_empty() {
            "Tanpa judul"
        } else {
            summary_fallback
        }
    } else {
        article.title.trim()
    };

    if article.url.trim().is_empty() {
        format!("➡️ **{title}**")
    } else {
        format!(
            "➡️ **{title}** - [Read Article]({})",
            sanitize_markdown_url(&article.url)
        )
    }
}

fn sanitize_markdown_url(url: &str) -> String {
    url.trim().replace('(', "%28").replace(')', "%29")
}

async fn send_to_discord(
    http_client: &HttpClient,
    webhook_url: &str,
    embeds: &[Value],
) -> Result<(), Box<dyn std::error::Error>> {
    let payload = serde_json::to_string(&json!({
        "embeds": embeds
    }))?;

    let response = http_client
        .post(webhook_url)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!(
            "Gagal mengirim ringkasan ke Discord. Status HTTP: {}",
            response.status()
        )
        .into());
    }

    Ok(())
}
