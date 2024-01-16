/// utilities to interface with DBLP
use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::Deserialize;
use skim::prelude::*;
use std::borrow::Cow;

const URLS: [&str; 2] = ["https://dblp.org", "https://dblp.uni-trier.de"];

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum Format {
    Condensed,
    Standard,
}

impl Format {
    pub fn get_param(&self) -> &str {
        match self {
            Format::Standard => "?param=1",
            Format::Condensed => "?param=0",
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct DblpResponse {
    result: DblpResult,
}

impl DblpResponse {
    pub fn matches(&self) -> impl Iterator<Item = DblpHitInfo> + '_ {
        self.result.hits.hit.iter().map(|hit| hit.info.clone())
    }

    pub fn query(query: &str, bibformat: Format) -> Result<Self> {
        URLS.iter()
            .map(|url| {
                let url = format!(
                    "{}/search/publ/api?q={}&format=json&{}",
                    url,
                    query,
                    bibformat.get_param()
                );
                ureq::get(&url).call()
            })
            .find(|r| r.is_ok())
            .context("no successful response")??
            .into_json()
            .context("error converting from json")
    }
}

#[derive(Deserialize, Debug)]
pub struct DblpResult {
    hits: DblpHits,
}

#[derive(Deserialize, Debug)]
pub struct DblpHits {
    hit: Vec<DblpHit>,
}

#[derive(Deserialize, Debug)]
pub struct DblpHit {
    info: DblpHitInfo,
}

#[derive(Deserialize, Debug, Clone)]
pub struct DblpHitInfo {
    pub key: String,
    pub authors: DblpAuthorEntry,
    pub title: String,
    pub venue: String,
    pub year: String,
    pub url: String,
}

impl DblpHitInfo {
    pub fn bib_url(&self, bibtype: Format) -> String {
        match bibtype {
            Format::Standard => format!("{}.bib?param=1", self.url),
            Format::Condensed => format!("{}.bib?param=0", self.url),
        }
    }

    pub fn get_key(&self) -> String {
        format!("DBLP:{}", self.key)
    }
}

pub fn bold(s: &str) -> String {
    format!("\x1b[1m{}\x1b[0m", s)
}

pub fn underline(s: &str) -> String {
    format!("\x1b[4m{}\x1b[0m", s)
}

impl SkimItem for DblpHitInfo {
    fn text(&self) -> Cow<'_, str> {
        Cow::Owned(format!(
            "{} {}",
            self.title,
            self.authors.as_vec().join(" ")
        ))
    }

    fn display<'a>(&'a self, _context: DisplayContext<'a>) -> AnsiString<'a> {
        AnsiString::from(self.title.clone())
    }

    fn preview(&self, _context: PreviewContext) -> ItemPreview {
        ItemPreview::AnsiText(format!(
            "{}\n{}\n{} {}",
            underline(&self.authors.as_vec().join(", ")),
            bold(&self.title),
            self.venue,
            self.year
        ))
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct DblpAuthor {
    #[serde(rename = "text")]
    pub name: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct DblpAuthorEntry {
    pub author: DblpAuthorList,
}

impl DblpAuthorEntry {
    pub fn as_vec(&self) -> Vec<String> {
        match &self.author {
            DblpAuthorList::Single(author) => vec![author.name.clone()],
            DblpAuthorList::List(authors) => authors.iter().map(|a| a.name.clone()).collect(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum DblpAuthorList {
    Single(DblpAuthor),
    List(Vec<DblpAuthor>),
}
