use anyhow::{anyhow, bail, Context, Result};
use clap::{App, Arg};
use serde::Deserialize;
use skim::prelude::*;
use std::{fs::File, io::BufReader, path::PathBuf};
use std::{fs::OpenOptions, io::prelude::*};

#[derive(Deserialize, Debug)]
struct DblpResponse {
    result: DblpResult,
}

impl DblpResponse {
    fn matches(&self) -> impl Iterator<Item = DblpHitInfo> + '_ {
        self.result.hits.hit.iter().map(|hit| hit.info.clone())
    }
}

#[derive(Deserialize, Debug)]
struct DblpResult {
    query: String,
    hits: DblpHits,
}

#[derive(Deserialize, Debug)]
struct DblpHits {
    hit: Vec<DblpHit>,
}

#[derive(Deserialize, Debug)]
struct DblpHit {
    url: String,
    info: DblpHitInfo,
}

#[derive(Deserialize, Debug, Clone)]
struct DblpHitInfo {
    key: String,
    authors: DblpAuthorEntry,
    title: String,
    venue: String,
    volume: Option<String>,
    number: Option<String>,
    year: String,
    #[serde(rename = "type")]
    entry_type: String,
    url: String,
}

impl DblpHitInfo {
    fn bib_url(&self) -> String {
        format!("{}.bib", self.url)
    }

    fn get_key(&self) -> String {
        format!("DBLP:{}", self.key)
    }
}

fn bold(s: &str) -> String {
    format!("\x1b[1m{}\x1b[0m", s)
}

fn underline(s: &str) -> String {
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
struct DblpAuthor {
    #[serde(rename = "text")]
    name: String,
}

#[derive(Deserialize, Debug, Clone)]
struct DblpAuthorEntry {
    author: DblpAuthorList,
}

impl DblpAuthorEntry {
    fn as_vec(&self) -> Vec<String> {
        match &self.author {
            DblpAuthorList::Single(author) => vec![author.name.clone()],
            DblpAuthorList::List(authors) => authors.iter().map(|a| a.name.clone()).collect(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum DblpAuthorList {
    Single(DblpAuthor),
    List(Vec<DblpAuthor>),
}

fn main() -> Result<()> {
    let matches = App::new("dblp")
        .version("0.1")
        .author("Matteo Ceccarello")
        .about("Easily query DBLP from the command line")
        .arg(
            Arg::with_name("bibtex")
                .short("b")
                .takes_value(true)
                .required(true),
        )
        .arg(Arg::with_name("query").multiple(true).required(true))
        .get_matches();

    let query: Vec<&str> = matches
        .values_of("query")
        .context("missing query")?
        .flat_map(|v| v.split(" "))
        .map(|v| v.trim())
        .collect();
    let query = query.join("+");

    let resp: DblpResponse = ureq::get(&format!(
        "http://dblp.org/search/publ/api?q={}&format=json",
        query
    ))
    .call()?
    .into_json()?;

    let selection = show_and_select(resp.matches())?;

    let bib_path = PathBuf::from(matches.value_of("bibtex").context("missing bibtex file")?);
    if !is_present(&bib_path, &selection)? {
        let bib = ureq::get(&selection.bib_url()).call()?.into_string()?;
        let mut writer = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&bib_path)?;
        writeln!(writer, "{}", bib)?;
    }

    Ok(())
}

fn is_present(path: &PathBuf, item: &DblpHitInfo) -> Result<bool> {
    let bib_key = item.get_key();

    if path.is_file() {
        let reader = BufReader::new(File::open(&path)?);
        for line in reader.lines() {
            let line = line?;
            if line.contains(&bib_key) {
                return Ok(true);
            }
        }
    }
    return Ok(false);
}

// copied from https://github.com/Mountlex/xivar/blob/main/src/finder.rs
fn show_and_select<I, T>(iter: T) -> Result<I>
where
    T: Iterator<Item = I>,
    I: SkimItem + Clone,
{
    let options = SkimOptionsBuilder::default()
        .height(Some("100%"))
        .preview(Some(""))
        .build()
        .expect("building fuzzy selector");

    let (tx_item, rx_item): (SkimItemSender, SkimItemReceiver) = unbounded();
    for item in iter {
        let _ = tx_item.send(Arc::new(item));
    }

    drop(tx_item); // so that skim could know when to stop waiting for more items.

    if let Some(output) = Skim::run_with(&options, Some(rx_item)) {
        if !output.is_abort {
            output
                .selected_items
                .into_iter()
                .nth(0)
                .map(move |item| {
                    (*item)
                        .as_any()
                        .downcast_ref::<I>() // downcast to concrete type
                        .expect("something wrong with downcast")
                        .clone()
                })
                .ok_or(anyhow!("Internal error"))
        } else {
            bail!("No entry selected! Aborting...")
        }
    } else {
        bail!("Internal error")
    }
}
