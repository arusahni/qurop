use clap::{Parser, Subcommand, ValueEnum};

use crate::config;

impl ValueEnum for config::WindowMatcher {
    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self {
            config::WindowMatcher::Process => Some("process".into()),
            config::WindowMatcher::Class => Some("class".into()),
        }
    }

    fn value_variants<'a>() -> &'a [Self] {
        &[config::WindowMatcher::Class, config::WindowMatcher::Process]
    }

    fn from_str(input: &str, _ignore_case: bool) -> Result<Self, String> {
        match input.to_lowercase().as_str() {
            "process" => Ok(Self::Process),
            "class" => Ok(Self::Class),
            _ => Err("Invalid value".into()),
        }
    }
}

/// Actions to carry out
#[derive(Clone, Debug, Subcommand)]
#[clap(rename_all = "kebab-case")]
pub enum Command {
    /// Open an application instance
    Open {
        /// the application instance to open
        name: String,
    },
    /// Add an application instance to the configuration
    Add {
        /// The method qurop should use to track the window instance.
        #[arg(long, value_enum, default_value_t = config::WindowMatcher::Process)]
        matcher: config::WindowMatcher,
        #[arg(long, value_enum)]
        class_name: Option<String>,
        /// the name of the application instance
        #[arg(required = true)]
        name: String,
        /// the command to execute
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,
    },
    /// Kill the active session associated with the instance, terminating the application.
    Kill {
        /// the name of the application instance
        name: String,
    },
    /// Hide an application instance
    Hide {
        /// the name of the application instance
        name: String,
    },
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}
