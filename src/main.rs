use crossterm::{
    event::{self, Event as CEvent, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;
use tui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{
        Block, BorderType, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table, Tabs,
    },
    Terminal,
};

const DB_PATH: &str = "./data/db.json";

#[derive(Error, Debug)]
pub enum Error {
    #[error("error reading the DB file: {0}")]
    ReadDBError(#[from] io::Error),
    #[error("error parsing the DB file: {0}")]
    ParseDBError(#[from] serde_json::Error),
}

enum Event<I> {
    Input(I),
    Tick,
}

#[derive(Serialize, Deserialize, Clone)]
struct Group {
    name: String,
    assignment: usize,
    feedback: Vec<String>,
    footnote: String,
}

#[derive(Copy, Clone, Debug)]
enum MenuItem {
    Home,
    Groups,
    Editing,
}

enum InputMode {
    Normal,
    // Editing,
}

/// App holds the state of the application
struct App {
    /// Current value of the input box
    input: String,
    /// Current input mode
    input_mode: InputMode,
    /// History of recorded messages
    messages: Vec<String>,
}

impl Default for App {
    fn default() -> App {
        App {
            input: String::new(),
            input_mode: InputMode::Normal,
            messages: Vec::new(),
        }
    }
}

impl From<MenuItem> for usize {
    fn from(input: MenuItem) -> usize {
        match input {
            MenuItem::Home => 0,
            MenuItem::Groups => 1,
            MenuItem::Editing => 2,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode().expect("can run in raw mode");

    let (tx, rx) = mpsc::channel();
    let tick_rate = Duration::from_millis(200);
    thread::spawn(move || {
        let mut last_tick = Instant::now();
        loop {
            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if event::poll(timeout).expect("poll works") {
                if let CEvent::Key(key) = event::read().expect("can read events") {
                    tx.send(Event::Input(key)).expect("can send events");
                }
            }

            if last_tick.elapsed() >= tick_rate {
                if let Ok(_) = tx.send(Event::Tick) {
                    last_tick = Instant::now();
                }
            }
        }
    });

    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let menu_titles = vec!["Home", "Groups", "Add", "Delete", "Quit"];
    let mut active_menu_item = MenuItem::Home;
    let mut group_list_state = ListState::default();
    group_list_state.select(Some(0));

    loop {
        terminal.draw(|rect| {
            let size = rect.size();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(2)
                .constraints(
                    [
                        Constraint::Length(3),
                        Constraint::Min(2),
                        Constraint::Length(3),
                    ]
                    .as_ref(),
                )
                .split(size);

            let copyright = Paragraph::new("SAGrading - CLI 2021")
                .style(Style::default().fg(Color::LightCyan))
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .style(Style::default().fg(Color::White))
                        .title("Copyright")
                        .border_type(BorderType::Plain),
                );

            let menu = menu_titles
                .iter()
                .map(|t| {
                    let (first, rest) = t.split_at(1);
                    Spans::from(vec![
                        Span::styled(
                            first,
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::UNDERLINED),
                        ),
                        Span::styled(rest, Style::default().fg(Color::White)),
                    ])
                })
                .collect();

            let tabs = Tabs::new(menu)
                .select(active_menu_item.into())
                .block(Block::default().title("Menu").borders(Borders::ALL))
                .style(Style::default().fg(Color::White))
                .highlight_style(Style::default().fg(Color::Yellow))
                .divider(Span::raw("|"));

            rect.render_widget(tabs, chunks[0]);
            match active_menu_item {
                MenuItem::Home => rect.render_widget(render_home(), chunks[1]),
                MenuItem::Groups => {
                    let groups_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints(
                            [Constraint::Percentage(20), Constraint::Percentage(80)].as_ref(),
                        )
                        .split(chunks[1]);
                    let (left, right) = render_groups(&group_list_state);
                    rect.render_stateful_widget(left, groups_chunks[0], &mut group_list_state);
                    rect.render_widget(right, groups_chunks[1]);
                }
                MenuItem::Editing => rect.render_widget(render_home(), chunks[1]),
            }
            rect.render_widget(copyright, chunks[2]);
        })?;

        match rx.recv()? {
            Event::Input(event) => match event.code {
                KeyCode::Char('q') => {
                    disable_raw_mode()?;
                    terminal.show_cursor()?;
                    terminal.clear()?;
                    break;
                }
                KeyCode::Char('h') => active_menu_item = MenuItem::Home,
                KeyCode::Char('g') => active_menu_item = MenuItem::Groups,
                KeyCode::Char('a') => {
                    add_random_group_to_db().expect("can add new random group");
                }
                KeyCode::Char('e') => active_menu_item = MenuItem::Editing,
                KeyCode::Char('d') => {
                    remove_group_at_index(&mut group_list_state).expect("can remove group");
                }
                KeyCode::Down => {
                    if let Some(selected) = group_list_state.selected() {
                        let amount_groups = read_db().expect("can fetch group list").len();
                        if selected >= amount_groups - 1 {
                            group_list_state.select(Some(0));
                        } else {
                            group_list_state.select(Some(selected + 1));
                        }
                    }
                }
                KeyCode::Up => {
                    if let Some(selected) = group_list_state.selected() {
                        let amount_groups = read_db().expect("can fetch group list").len();
                        if selected > 0 {
                            group_list_state.select(Some(selected - 1));
                        } else {
                            group_list_state.select(Some(amount_groups - 1));
                        }
                    }
                }
                _ => {}
            },
            Event::Tick => {}
        }
    }

    Ok(())
}

fn render_home<'a>() -> Paragraph<'a> {
    let home = Paragraph::new(vec![
        Spans::from(vec![Span::raw("")]),
        Spans::from(vec![Span::raw("Welcome")]),
        Spans::from(vec![Span::raw("")]),
        Spans::from(vec![Span::raw("to")]),
        Spans::from(vec![Span::raw("")]),
        Spans::from(vec![Span::styled(
            "SAGrading-CLI",
            Style::default().fg(Color::LightBlue),
        )]),
        Spans::from(vec![Span::raw("")]),
        Spans::from(vec![Span::raw("--------------------------------------------------------------------------------------------------------")]),
        Spans::from(vec![Span::raw("< Press 'g' to access groups, 'a' to add random new groups and 'd' to delete the currently selected group. >")]),
        Spans::from(vec![Span::raw("--------------------------------------------------------------------------------------------------------")]),
        Spans::from(vec![Span::raw("    \\/")]),
        Spans::from(vec![Span::styled("|\\---/|", Style::default().fg(Color::LightBlue),)]),
        Spans::from(vec![Span::styled("| o_o |", Style::default().fg(Color::LightBlue),)]),
        Spans::from(vec![Span::styled(" \\_^_/ ", Style::default().fg(Color::LightBlue),)]),

    ])
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White))
            .title("Home")
            .border_type(BorderType::Plain),
    );
    home
}

fn render_groups<'a>(group_list_state: &ListState) -> (List<'a>, Table<'a>) {
    let groups = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::White))
        .title("Groups")
        .border_type(BorderType::Plain);

    let group_list = read_db().expect("can fetch group list");
    let items: Vec<_> = group_list
        .iter()
        .map(|group| {
            ListItem::new(Spans::from(vec![Span::styled(
                group.name.clone(),
                Style::default(),
            )]))
        })
        .collect();

    let selected_group = group_list
        .get(
            group_list_state
                .selected()
                .expect("there is always a selected group"),
        )
        .expect("exists")
        .clone();

    let list = List::new(items).block(groups).highlight_style(
        Style::default()
            .bg(Color::Yellow)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
    );

    let group_detail = Table::new(vec![Row::new(vec![
        Cell::from(Span::raw(selected_group.name.to_string())),
        Cell::from(Span::raw(selected_group.assignment.to_string())),
        Cell::from(Span::raw(selected_group.feedback.concat())),
    ])])
    .header(Row::new(vec![
        Cell::from(Span::styled(
            "Type",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Assignment",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Feedback",
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White))
            .title("Detail")
            .border_type(BorderType::Plain),
    )
    .widths(&[
        Constraint::Percentage(10),
        Constraint::Percentage(20),
        Constraint::Percentage(100),
    ]);

    (list, group_detail)
}

fn read_db() -> Result<Vec<Group>, Error> {
    let db_content = fs::read_to_string(DB_PATH)?;
    let parsed: Vec<Group> = serde_json::from_str(&db_content)?;
    Ok(parsed)
}

fn add_random_group_to_db() -> Result<Vec<Group>, Error> {
    let mut rng = rand::thread_rng();
    let db_content = fs::read_to_string(DB_PATH)?;
    let mut parsed: Vec<Group> = serde_json::from_str(&db_content)?;
    let mut textvector = Vec::new();
    let bottomtext = "This assignment was graded by: Tom Koning. E-mail: tom.koning@ru.nl.";
    textvector.push("feedback".to_string());

    let random_group = Group {
        name: format!("Group {}", rng.gen_range(0, 10).to_string()),
        assignment: rng.gen_range(0, 10),
        feedback: textvector,
        footnote: bottomtext.to_string(),
    };

    parsed.push(random_group);
    fs::write(DB_PATH, &serde_json::to_vec(&parsed)?)?;
    Ok(parsed)
}

fn remove_group_at_index(group_list_state: &mut ListState) -> Result<(), Error> {
    if let Some(selected) = group_list_state.selected() {
        let db_content = fs::read_to_string(DB_PATH)?;
        let mut parsed: Vec<Group> = serde_json::from_str(&db_content)?;
        if parsed.len() != 1 {
            parsed.remove(selected);
            fs::write(DB_PATH, &serde_json::to_vec(&parsed)?)?;
            if selected != 0 {
                group_list_state.select(Some(selected - 1));
            }
        }
    }
    Ok(())
}
