use anyhow::{anyhow, bail, Context, Result};
use biblatex::*;
use clap::{Parser, Subcommand};
use skim::prelude::*;
use std::{fs::File, io::BufReader, path::PathBuf};
use std::{fs::OpenOptions, io::prelude::*};

mod dblp;
use crate::dblp::*;

/// gets the path to the only bibtex file in a directory. If there is none
/// or if there are multiple, return None
fn get_unique_bib() -> Result<Option<PathBuf>> {
    let paths: Vec<PathBuf> = std::fs::read_dir(".")?
        .filter_map(|s| {
            let p = s.as_ref().unwrap().path();
            if let Some(ext) = p.extension() {
                if ext == "bib" {
                    return Some(p);
                }
            }
            None
        })
        .collect();

    if paths.len() == 1 {
        Ok(Some(paths[0].clone()))
    } else {
        Ok(None)
    }
}

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    subcommand: Actions,

    #[arg(short, long, value_name = "FILE")]
    bibtex: Option<String>,
}

impl Cli {
    fn get_bib_path(&self) -> Result<PathBuf> {
        self.bibtex
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| get_unique_bib().unwrap())
            .context("missing bibtex file")
    }

    fn get_backup_bib_path(&self) -> Result<PathBuf> {
        let orig = self.get_bib_path()?;
        Ok(orig.with_extension("bib.bak"))
    }
}

#[derive(Subcommand)]
enum Actions {
    Add { query: Vec<String> },
    Clip { query: Vec<String> },
    Convert { to: Format },
}

fn join_param_string(strings: &[String]) -> String {
    strings
        .iter()
        .flat_map(|v| v.split(' '))
        .map(|v| v.trim())
        .collect::<Vec<&str>>()
        .join("+")
}

fn write_clipboard(what: &str) -> Result<()> {
    fn run(cmd: &str, what: &str) -> Result<()> {
        let mut child = std::process::Command::new(cmd)
            .stdin(std::process::Stdio::piped())
            .spawn()?;
        write!(child.stdin.take().context("no standard input")?, "{}", what)?;
        Ok(())
    }
    ["wl-copy", "pbcopy"]
        .iter()
        .map(|cmd| run(cmd, what))
        .next()
        .context("no clipboard command ran successfully")?
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let bib_path = cli.get_bib_path();

    match cli.subcommand {
        Actions::Add { query } => {
            let bib_path = bib_path?;
            let query = join_param_string(&query);
            let bibformat = Format::Condensed;
            let resp = DblpResponse::query(&query, bibformat)?;
            let selection = show_and_select(resp.matches())?;

            if !is_present(&bib_path, &selection)? {
                let bib = ureq::get(&selection.bib_url(Format::Standard))
                    .call()?
                    .into_string()?;
                let mut writer = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&bib_path)?;
                writeln!(writer, "{}", bib)?;
            }
            write_clipboard(&format!("DBLP:{}", selection.key))?;
        }
        Actions::Clip { query } => {
            let query = join_param_string(&query);
            let bibformat = Format::Condensed;
            let resp = DblpResponse::query(&query, bibformat)?;

            let selection = show_and_select(resp.matches())?;
            let bib = ureq::get(&selection.bib_url(Format::Standard))
                .call()?
                .into_string()?;
            write_clipboard(&bib)?;
        }
        Actions::Convert { to } => {
            let bib_path = bib_path?;
            let mut f = File::open(&bib_path)?;
            let mut src = String::new();
            f.read_to_string(&mut src)?;
            drop(f);

            // backup the content
            let mut f = File::create(cli.get_backup_bib_path()?)?;
            writeln!(f, "{}", src)?;
            drop(f);

            // overwrite the file
            let mut f = File::create(bib_path)?;

            let bibliography = Bibliography::parse(&src).unwrap();
            for entry in bibliography.iter() {
                let bibstr = entry.to_bibtex_string().map_err(|e| anyhow!(e))?;
                eprintln!("{}", entry.key);
                if entry.key.starts_with("DBLP") {
                    let k = entry.key.replace("DBLP:", "");
                    let url = format!("https://dblp.uni-trier.de/rec/{}.bib{}", k, to.get_param());
                    if let Err(err) = ureq::get(&url)
                        .call()
                        .and_then(|res| Ok(res.into_string()?))
                        .and_then(|bib| Ok(writeln!(f, "{}\n", bib)?))
                        .or_else(|_| writeln!(f, "{}\n", bibstr))
                    {
                        eprintln!("Error in fetching data for {}: {:?}", entry.key, err);
                    }
                } else {
                    writeln!(f, "{}\n", bibstr)?;
                }
            }
        }
    }

    Ok(())
}

fn is_present(path: &PathBuf, item: &DblpHitInfo) -> Result<bool> {
    let bib_key = item.get_key();

    if path.is_file() {
        let reader = BufReader::new(File::open(path)?);
        for line in reader.lines() {
            let line = line?;
            if line.contains(&bib_key) {
                return Ok(true);
            }
        }
    }
    Ok(false)
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
                .next()
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
