use biblatex::*;
use anyhow::{anyhow, bail, Context, Result};
use clap::{Arg, Command};
use copypasta::{ClipboardContext, ClipboardProvider};
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
    hits: DblpHits,
}

#[derive(Deserialize, Debug)]
struct DblpHits {
    hit: Vec<DblpHit>,
}

#[derive(Deserialize, Debug)]
struct DblpHit {
    info: DblpHitInfo,
}

enum BibType {
    Standard,
    Condensed,
}

#[derive(Deserialize, Debug, Clone)]
struct DblpHitInfo {
    key: String,
    authors: DblpAuthorEntry,
    title: String,
    venue: String,
    year: String,
    url: String,
}

impl DblpHitInfo {
    fn bib_url(&self, bibtype: BibType) -> String {
        match bibtype {
            BibType::Standard => format!("{}.bib?param=1", self.url),
            BibType::Condensed => format!("{}.bib?param=0", self.url),
        }
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
    let convert_str = "convert".to_owned();
    if let Some("convert") = std::env::args().nth(1).as_ref().map(|s| s.as_ref()) {
        let matches = Command::new("convert")
            .arg(
                Arg::new("bibtex")
                    .short('b')
                    .takes_value(true)
                    .required(true),
            )
            .arg(
                Arg::new("to")
                    .takes_value(true)
                    .required(true)
                    .possible_values(&["condensed", "standard"])
            ).get_matches_from(std::env::args().filter(|a| a != &convert_str)); 
        let bibtype = match matches.value_of("to").unwrap() {
            "condensed" => BibType::Condensed,
            "standard" => BibType::Standard,
            _ => panic!()
        };

        let bib_path = PathBuf::from(matches.value_of("bibtex").context("missing bibtex file")?);
        let mut f = File::open(bib_path)?;
        let mut src = String::new();
        f.read_to_string(&mut src)?;
        let bibliography = Bibliography::parse(&src).unwrap();
        for entry in bibliography.iter() {
            if entry.key.starts_with("DBLP") {
                let k = entry.key.replace("DBLP:", "");
                let param = match bibtype {
                    BibType::Standard => "?param=1",
                    BibType::Condensed => "?param=0",
                };
                let url = format!("https://dblp.org/rec/{}.bib{}", k, param);
                let bib = ureq::get(&url).call()?.into_string()?;
                println!("{}\n", bib);
            } else {
                println!("{}\n", entry.to_bibtex_string());
            }
        }


        return Ok(())
    }
    let matches = Command::new("dblp")
        .version("0.1")
        .author("Matteo Ceccarello")
        .about("Easily query DBLP from the command line")
        .arg(
            Arg::new("bibtex")
                .short('b')
                .takes_value(true)
                .required(true),
        )
        .arg(Arg::new("query").multiple_values(true).required(true))
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
        let bib = ureq::get(&selection.bib_url(BibType::Standard)).call()?.into_string()?;
        let mut writer = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&bib_path)?;
        writeln!(writer, "{}", bib)?;
    }

    // Put the key in the clipboard
    ClipboardContext::new()
        .map_err(|e| anyhow!("getting the clipboard: {}", e))?
        .set_contents(format!("DBLP:{}", selection.key))
        .map_err(|e| anyhow!("pasting to clipboard: {}", e))?;

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
