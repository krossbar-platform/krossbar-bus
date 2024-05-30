use std::sync::{Arc, Mutex};

use rustyline::{hint::Hinter, Completer, Helper as RustyHelper, Highlighter, Validator};

/// Strcut, to track current user input
#[derive(Clone, Completer, Highlighter, Validator)]
pub struct Helper {
    current_input: Arc<Mutex<String>>,
}

impl Helper {
    pub fn new() -> Self {
        Self {
            current_input: Arc::new(Mutex::new("".into())),
        }
    }

    pub fn get_user_input(&self) -> String {
        self.current_input.lock().unwrap().clone()
    }
}

impl Hinter for Helper {
    type Hint = String;

    fn hint(&self, line: &str, _: usize, _: &rustyline::Context<'_>) -> Option<Self::Hint> {
        *self.current_input.lock().unwrap() = line.into();
        None
    }
}

impl RustyHelper for Helper {}
