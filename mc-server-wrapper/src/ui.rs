use crossterm::event::{Event, KeyCode};
use ringbuffer::RingBuffer;
use textwrap::Wrapper;
use tui::backend::Backend;
use tui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, List, Paragraph, Tabs, Text},
    Frame,
};

/// Represents the current state of the terminal UI
#[derive(Debug)]
pub struct TuiState {
    pub tab_state: TabsState,
    pub logs_state: LogsState,
    pub input_state: InputState,
}

impl TuiState {
    pub fn new() -> Self {
        TuiState {
            tab_state: TabsState::new(vec!["Logs".into()]),
            logs_state: LogsState {
                records: RingBuffer::with_capacity(512),
            },
            input_state: InputState { value: "".into() },
        }
    }

    /// Draw the current state to the given frame
    pub fn draw<B: Backend>(&self, f: &mut Frame<B>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(2),
                    Constraint::Min(0),
                    Constraint::Length(2),
                ]
                .as_ref(),
            )
            .split(f.size());

        self.tab_state.draw(f, chunks[0]);
        self.logs_state.draw(f, chunks[1]);
        self.input_state.draw(f, chunks[2]);
    }

    /// Update the state based on the given input
    // TODO: have handle_input for each state struct?
    pub fn handle_input(&mut self, event: Event) {
        if let Event::Key(key_event) = event {
            match key_event.code {
                KeyCode::Char(c) => self.input_state.value.push(c),
                KeyCode::Backspace => {
                    self.input_state.value.pop();
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug)]
pub struct TabsState {
    /// List of tab titles
    titles: Vec<String>,
    /// The currently active tab
    current_idx: usize,
}

impl TabsState {
    /// Draw the current state in the given `area`
    fn draw<B: Backend>(&self, f: &mut Frame<B>, area: Rect) {
        let tabs = Tabs::default()
            .block(Block::default().borders(Borders::BOTTOM))
            .titles(&self.titles)
            .style(Style::default().fg(Color::Green))
            .highlight_style(Style::default().fg(Color::Yellow))
            .select(self.current_idx);

        f.render_widget(tabs, area);
    }

    fn new(titles: Vec<String>) -> Self {
        Self {
            titles,
            current_idx: 0,
        }
    }

    // /// Change to the next tab
    // pub fn next(&mut self) {
    //     self.current_idx = (self.current_idx + 1) % self.titles.len();
    // }

    // /// Change to the previous tab
    // pub fn previous(&mut self) {
    //     if self.current_idx > 0 {
    //         self.current_idx -= 1;
    //     } else {
    //         self.current_idx = self.titles.len() - 1;
    //     }
    // }
}

#[derive(Debug)]
pub struct LogsState {
    /// Stores the log messages to be displayed
    records: RingBuffer<Text<'static>>,
}

impl LogsState {
    /// Draw the current state in the given `area`
    fn draw<B: Backend>(&self, f: &mut Frame<B>, area: Rect) {
        let area_height = area.height as usize;
        let wrapper = Wrapper::new(area.width as usize);

        // TODO: this does a lot more wrapping than it needs to depending on
        // the situation
        let wrapped_lines: Vec<Text> = self
            .records
            .iter()
            // Only process the records we could potentially be displaying
            .skip(self.records.len().saturating_sub(area_height))
            .map(|r| {
                if let Text::Raw(text) = r {
                    wrapper.wrap(text.as_ref()).into_iter().map(Text::raw)
                } else {
                    unreachable!()
                }
            })
            .flatten()
            .collect();

        // TODO: we should be wrapping text with paragraph, but it currently
        // doesn't support wrapping and staying scrolled to the bottom
        //
        // see https://github.com/fdehau/tui-rs/issues/89
        let logs = List::new(
            wrapped_lines
                .iter()
                .skip(wrapped_lines.len().saturating_sub(area_height))
                .cloned(),
        )
        .block(Block::default().borders(Borders::NONE));
        f.render_widget(logs, area);
    }

    /// Add a record to be displayed
    pub fn add_record(&mut self, record: Text<'static>) {
        self.records.push(record);
    }
}

#[derive(Debug)]
pub struct InputState {
    /// The current value of the input
    value: String,
}

impl InputState {
    /// Draw the current state in the given `area`
    fn draw<B: Backend>(&self, f: &mut Frame<B>, area: Rect) {
        let text = [Text::raw("> "), Text::raw(&self.value)];
        let input = Paragraph::new(text.iter())
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::NONE));

        f.render_widget(input, area);
    }

    /// Clear the input
    pub fn clear(&mut self) {
        self.value.clear();
    }

    /// Get the current value of the input
    pub fn value(&self) -> String {
        self.value.clone()
    }
}
