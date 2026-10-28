use iced::widget::text::highlighter::{Style, Underline};
use iced::widget::{
    button, center_x, column, container, operation, pick_list, row, space, text, text_editor,
    toggler, tooltip,
};
use iced::{Center, Element, Fill, Font, Task, Theme, Window};
use iced::{Color, window};
use iced::{highlighter, keyboard};

use std::collections::HashMap;
use std::ffi;
use std::io;
use std::ops::Deref;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn main() -> iced::Result {
    iced::application(Editor::new, Editor::update, Editor::view)
        .theme(Editor::theme)
        .fonts([include_bytes!("../fonts/icons.ttf").as_slice()])
        .font(Font::MONOSPACE)
        .run()
}

#[derive(Debug, Clone, Default)]
pub struct ErrorMap(Arc<HashMap<usize, Vec<Range<usize>>>>);

impl ErrorMap {
    pub fn new(map: HashMap<usize, Vec<Range<usize>>>) -> Self {
        Self(Arc::new(map))
    }
}

impl PartialEq for ErrorMap {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Deref for ErrorMap {
    type Target = HashMap<usize, Vec<Range<usize>>>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

struct Editor {
    file: Option<PathBuf>,
    content: text_editor::Content,
    theme: Theme,
    word_wrap: bool,
    is_loading: bool,
    is_dirty: bool,
    error_map: ErrorMap,
}

#[derive(Debug, Clone)]
enum Message {
    ActionPerformed(text_editor::Action),
    ThemeSelected(Theme),
    WordWrapToggled(bool),
    NewFile,
    OpenFile,
    FileOpened(Result<(PathBuf, Arc<String>), Error>),
    SaveFile,
    FileSaved(Result<PathBuf, Error>),
}

impl Editor {
    fn new() -> (Self, Task<Message>) {
        (
            Self {
                file: None,
                content: text_editor::Content::new(),
                theme: Theme::CatppuccinMocha,
                word_wrap: true,
                is_loading: true,
                is_dirty: false,
                error_map: ErrorMap::new(HashMap::from([(0, vec![0..16; 1])])),
            },
            Task::batch([
                Task::perform(
                    load_file(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs",)),
                    Message::FileOpened,
                ),
                operation::focus(EDITOR),
            ]),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ActionPerformed(action) => {
                self.is_dirty = self.is_dirty || action.is_edit();

                self.content.perform(action);

                Task::none()
            }
            Message::ThemeSelected(theme) => {
                self.theme = theme;

                Task::none()
            }
            Message::WordWrapToggled(word_wrap) => {
                self.word_wrap = word_wrap;

                Task::none()
            }
            Message::NewFile => {
                if !self.is_loading {
                    self.file = None;
                    self.content = text_editor::Content::new();
                    self.error_map = ErrorMap::default();
                }

                Task::none()
            }
            Message::OpenFile => {
                if self.is_loading {
                    Task::none()
                } else {
                    self.is_loading = true;

                    window::oldest()
                        .and_then(|id| window::run(id, open_file))
                        .then(Task::future)
                        .map(Message::FileOpened)
                }
            }
            Message::FileOpened(result) => {
                self.is_loading = false;
                self.is_dirty = false;

                if let Ok((path, contents)) = result {
                    self.file = Some(path);
                    self.content = text_editor::Content::with_text(&contents);
                }

                Task::none()
            }
            Message::SaveFile => {
                if self.is_loading {
                    Task::none()
                } else {
                    self.is_loading = true;

                    let mut text = self.content.text();

                    if let Some(ending) = self.content.line_ending()
                        && !text.ends_with(ending.as_str())
                    {
                        text.push_str(ending.as_str());
                    }

                    Task::perform(save_file(self.file.clone(), text), Message::FileSaved)
                }
            }
            Message::FileSaved(result) => {
                self.is_loading = false;

                if let Ok(path) = result {
                    self.file = Some(path);
                    self.is_dirty = false;
                }

                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let controls = row![
            action(new_icon(), "New file", Some(Message::NewFile)),
            action(
                open_icon(),
                "Open file",
                (!self.is_loading).then_some(Message::OpenFile)
            ),
            action(
                save_icon(),
                "Save file",
                self.is_dirty.then_some(Message::SaveFile)
            ),
            space::horizontal(),
            toggler(self.word_wrap)
                .label("Word Wrap")
                .on_toggle(Message::WordWrapToggled),
            pick_list(Some(&self.theme), Theme::ALL, Theme::to_string)
                .on_select(Message::ThemeSelected),
        ]
        .spacing(10)
        .align_y(Center);

        let status = row![
            text(if let Some(path) = &self.file {
                let path = path.display().to_string();

                if path.len() > 60 {
                    format!("...{}", &path[path.len() - 40..])
                } else {
                    path
                }
            } else {
                String::from("New file")
            }),
            space::horizontal(),
            text({
                let cursor = self.content.cursor();

                format!("{}:{}", cursor.position.line + 1, cursor.position.index + 1)
            })
        ]
        .spacing(10);

        column![
            controls,
            text_editor(&self.content)
                .id(EDITOR)
                .height(Fill)
                .on_action(Message::ActionPerformed)
                .wrapping(if self.word_wrap {
                    text::Wrapping::Word
                } else {
                    text::Wrapping::None
                })
                .highlight_with::<ConfigParser>(
                    Settings {
                        inner: highlighter::Settings {
                            token: self
                                .file
                                .as_deref()
                                .and_then(Path::extension)
                                .and_then(ffi::OsStr::to_str)
                                .unwrap_or("rs")
                                .to_owned(),
                        },
                        error_map: self.error_map.clone(),
                    },
                    config_highlight,
                )
                .key_binding(|key_press| {
                    match key_press.key.as_ref() {
                        keyboard::Key::Character("s") if key_press.modifiers.command() => {
                            Some(text_editor::Binding::Custom(Message::SaveFile))
                        }
                        _ => text_editor::Binding::from_key_press(key_press),
                    }
                }),
            status,
        ]
        .spacing(10)
        .padding(10)
        .into()
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Settings {
    inner: highlighter::Settings,
    error_map: ErrorMap,
}

enum Code {
    Syntax(iced::Code),
    Error(iced::Code),
}

// iced toml parser, with the config error lines.
struct ConfigParser {
    inner: highlighter::Parser,
    error_map: ErrorMap,
}

type ConfigCodeIterator<'a> = Box<dyn Iterator<Item = (Range<usize>, Code)> + 'a>;

impl ConfigParser {
    fn new(settings: &Settings) -> Self {
        Self {
            inner: highlighter::Parser::new(&settings.inner),
            error_map: settings.error_map.clone(),
        }
    }

    fn update(&mut self, settings: &Settings) {
        self.inner.update(&settings.inner);
        if self.error_map != settings.error_map {
            self.error_map = settings.error_map.clone();
        }
    }

    fn change_line(&mut self, line: usize) {
        self.inner.change_line(line);
    }

    fn parse_line(&mut self, line: &str) -> ConfigCodeIterator<'_> {
        let current_line = self.inner.current_line();
        let spans = self.inner.parse_line(line);

        let Some(errors) = self.error_map.get(&current_line) else {
            return Box::new(spans.map(|(range, code)| (range, Code::Syntax(code))));
        };

        let mut result = Vec::new();
        let mut err_idx = 0;

        for (range, code) in spans {
            let mut cursor = range.start;

            while cursor < range.end {
                while err_idx < errors.len() && errors[err_idx].end <= cursor {
                    err_idx += 1;
                }

                let (end, code) = match errors.get(err_idx) {
                    Some(err) if err.start <= cursor => (err.end.min(range.end), Code::Error(code)),
                    Some(err) => (err.start.min(range.end), Code::Syntax(code)),
                    None => (range.end, Code::Syntax(code)),
                };

                result.push((cursor..end, code));
                cursor = end;
            }
        }

        Box::new(result.into_iter())
    }

    fn current_line(&self) -> usize {
        self.inner.current_line()
    }
}

impl text::Parser for ConfigParser {
    type Settings = Settings;
    type Output = Code;
    type Iterator<'a> = ConfigCodeIterator<'a>;

    fn new(settings: &Self::Settings) -> Self {
        Self::new(settings)
    }

    fn update(&mut self, new_settings: &Self::Settings) {
        self.update(new_settings);
    }

    fn change_line(&mut self, line: usize) {
        self.change_line(line);
    }

    fn parse_line(&mut self, line: &str) -> Self::Iterator<'_> {
        self.parse_line(line)
    }

    fn current_line(&self) -> usize {
        self.current_line()
    }
}

fn config_highlight(output: Code, theme: &Theme) -> Style {
    match output {
        Code::Syntax(code) => code.highlight(theme),
        Code::Error(code) => Style {
            underline: Some(Underline::Single),
            underline_color: Some(Color::from_rgb(1.0, 0.0, 0.0)),
            strikethrough: true,
            strikethrough_color: Some(Color::from_rgb(0.0, 1.0, 0.0)),
            overline: true,
            overline_color: Some(Color::from_rgb(0.0, 0.0, 1.0)),
            ..code.highlight(theme)
        },
    }
}

#[derive(Debug, Clone)]
pub enum Error {
    DialogClosed,
    IoError(io::ErrorKind),
}

fn open_file(
    window: &dyn Window,
) -> impl Future<Output = Result<(PathBuf, Arc<String>), Error>> + use<> {
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Open a text file...")
        .set_parent(&window);

    async move {
        let picked_file = dialog.pick_file().await.ok_or(Error::DialogClosed)?;

        load_file(picked_file).await
    }
}

async fn load_file(path: impl Into<PathBuf>) -> Result<(PathBuf, Arc<String>), Error> {
    let path = path.into();

    let contents = tokio::fs::read_to_string(&path)
        .await
        .map(Arc::new)
        .map_err(|error| Error::IoError(error.kind()))?;

    Ok((path, contents))
}

async fn save_file(path: Option<PathBuf>, contents: String) -> Result<PathBuf, Error> {
    let path = if let Some(path) = path {
        path
    } else {
        rfd::AsyncFileDialog::new()
            .save_file()
            .await
            .as_ref()
            .map(rfd::FileHandle::path)
            .map(Path::to_owned)
            .ok_or(Error::DialogClosed)?
    };

    tokio::fs::write(&path, contents)
        .await
        .map_err(|error| Error::IoError(error.kind()))?;

    Ok(path)
}

fn action<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
    label: &'a str,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let action = button(center_x(content).width(30));

    if let Some(on_press) = on_press {
        tooltip(
            action.on_press(on_press),
            label,
            tooltip::Position::FollowCursor,
        )
        .style(container::rounded_box)
        .into()
    } else {
        action.style(button::secondary).into()
    }
}

fn new_icon<'a, Message>() -> Element<'a, Message> {
    icon('\u{0e800}')
}

fn save_icon<'a, Message>() -> Element<'a, Message> {
    icon('\u{0e801}')
}

fn open_icon<'a, Message>() -> Element<'a, Message> {
    icon('\u{0f115}')
}

fn icon<'a, Message>(codepoint: char) -> Element<'a, Message> {
    const ICON_FONT: Font = Font::new("editor-icons");

    text(codepoint)
        .font(ICON_FONT)
        .shaping(text::Shaping::Basic)
        .into()
}

const EDITOR: &str = "editor";
