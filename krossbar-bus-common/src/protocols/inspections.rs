use std::fmt::Display;

use colored::*;
use serde::{Deserialize, Serialize};

pub const INSPECT_METHOD: &str = "inspect";

#[derive(Serialize, Deserialize)]
pub struct InspectData {
    pub methods: Vec<String>,
    pub signals: Vec<String>,
    pub states: Vec<String>,
}

impl Display for InspectData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}:", "methods".bright_blue())?;
        self.methods
            .iter()
            .for_each(|method| writeln!(f, "\t{}", method).unwrap());

        writeln!(f, "{}:", "signals".bright_yellow())?;
        self.signals
            .iter()
            .for_each(|signal| writeln!(f, "\t{}", signal).unwrap());

        writeln!(f, "{}:", "states".bright_green())?;
        self.states
            .iter()
            .for_each(|state| writeln!(f, "\t{}", state).unwrap());

        Ok(())
    }
}
