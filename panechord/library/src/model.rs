//! Application snapshots contain no filesystem handles or worker pool.
use crate::scan::{Entry, Report};
use panechord_macros::commands;
use panechord_runtime::{Keyed, Viewport};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub folder: String,
    pub filter: String,
    pub revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub query: Query,
    pub generation: u64,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Row {
    pub label: String,
}
#[derive(Clone)]
pub struct App {
    pub folder: String,
    pub filter: String,
    pub revision: u64,
    pub generation: u64,
    pub pending: Option<Request>,
    pub entries: Arc<Vec<Entry>>,
    pub capped: bool,
    pub error: String,
    pub busy: bool,
    pub rows: Vec<Keyed<Row>>,
    pub viewport: Viewport,
    pub selection: usize,
    pub details: String,
    pub status: String,
    pub closing: bool,
}
impl App {
    pub fn new(folder: String) -> Self {
        Self {
            folder,
            filter: String::new(),
            revision: 0,
            generation: 0,
            pending: None,
            entries: Arc::new(Vec::new()),
            capped: false,
            error: String::new(),
            busy: true,
            rows: Vec::new(),
            viewport: Viewport {
                start: 0,
                count: 6,
                row_height: 32.0,
            },
            selection: 0,
            details: "No file selected".into(),
            status: "Starting…".into(),
            closing: false,
        }
    }
    pub fn query(&self) -> Query {
        Query {
            folder: self.folder.clone(),
            filter: self.filter.clone(),
            revision: self.revision,
        }
    }
    pub fn begin(&mut self, query: Query) {
        if self.query() != query {
            return;
        }
        self.generation = self
            .generation
            .checked_add(1)
            .expect("scan generation exhausted");
        self.pending = Some(Request {
            query,
            generation: self.generation,
        });
        self.busy = true;
        self.error.clear();
    }
    pub fn complete(&mut self, request: Request, result: Result<Report, String>) {
        if self.pending.as_ref() != Some(&request) || self.query() != request.query {
            return;
        }
        self.pending = None;
        self.busy = false;
        self.selection = 0;
        match result {
            Ok(mut report) => {
                // Defense in depth: even an injected scanner cannot grow the UI unboundedly.
                report.capped |= report.entries.len() > crate::scan::MAX_RESULTS;
                report.entries.truncate(crate::scan::MAX_RESULTS);
                self.entries = Arc::new(report.entries);
                self.capped = report.capped;
                self.error.clear();
            }
            Err(error) => {
                self.entries = Arc::new(Vec::new());
                self.capped = false;
                self.error = error;
            }
        }
    }
    fn can_select(&self) -> bool {
        !self.entries.is_empty() && !self.busy
    }
}
#[commands]
impl App {
    #[command]
    fn rescan(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("scan revision exhausted");
    }
    #[command(can_execute = "can_select")]
    fn next(&mut self) {
        self.selection = (self.selection + 1) % self.entries.len();
    }
    #[command]
    fn quit(&mut self) {
        self.closing = true;
    }
}
