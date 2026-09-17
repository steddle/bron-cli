//! The binary.
//!
//! Every command produces one value, and both outputs are written from it at
//! the very edge, so `--json` and human output cannot describe different
//! things. Where that value is the service's own answer, `--json` prints the
//! envelope as sent.
//!
//! On failure with `--json`, the error still goes to stdout as JSON and the
//! human sentence goes to stderr. A caller that has to grep stderr to tell a
//! missing article from a dead network will get it wrong eventually. The
//! failure envelope is bron's own, because `network` and `local` are kinds no
//! service can state.

mod cli;
mod render;
mod run;

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use clap::Parser;
use clap::error::ErrorKind;

use bron::error::{Fault, exit};
use bron::wire::Envelope;

use cli::Cli;
use run::Answer;

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        // clap exits 2 of its own accord, and 2 is not-found here. A command
        // line it will not accept is a usage failure, and `--help` and
        // `--version` are what a caller asked for rather than a failure at all.
        //
        // Bare `bron` is not among them. clap prints the help for it, but
        // nobody asked: the command line was incomplete, and a script reading
        // success for a command that did nothing is the failure that costs.
        Err(error) => {
            let asked_for = matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            );
            let _ = error.print();

            return if asked_for {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(exit::USAGE)
            };
        }
    };

    // Progress is never written when it could be mistaken for output, and never
    // when nobody is watching.
    let quiet = cli.quiet || cli.json || !io::stderr().is_terminal();

    match run::execute(&cli, quiet) {
        Ok(answer) => {
            if let Err(e) = emit(&answer, cli.json) {
                eprintln!("bron: cannot write output: {e}");
                return ExitCode::from(exit::LOCAL);
            }
            ExitCode::SUCCESS
        }
        Err(fault) => {
            report(&fault, cli.json);
            fault.exit_code()
        }
    }
}

fn emit(answer: &Answer, json: bool) -> io::Result<()> {
    let mut out = io::stdout().lock();
    if json {
        answer.to_json(&mut out)?;
        out.write_all(b"\n")
    } else {
        answer.render(&mut out)
    }
}

fn report(fault: &Fault, json: bool) {
    if json {
        // Still JSON on stdout, so a consumer parses one shape either way.
        if let Ok(text) = serde_json::to_string(&Envelope::failed(fault)) {
            println!("{text}");
        }
    }

    let chain = fault.chain();
    eprintln!(
        "bron: {}",
        chain.first().map(String::as_str).unwrap_or("failed")
    );
    for line in chain.iter().skip(1) {
        eprintln!("  caused by: {line}");
    }
}
