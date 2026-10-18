use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use ratatui::crossterm::event::KeyEventKind;
use ratatui::crossterm::event::KeyModifiers;

use crate::config::Config;
use crate::draft::Draft;
use crate::draft::Id;
use crate::draft::Kind;
use crate::draft::Page;
use crate::editor::Editor;
use crate::editor::Outcome;
use crate::i18n::tr;
use crate::i18n::tt;
use crate::input::Input;
use crate::plan;
use crate::resolve;
use crate::resolve::Package;

const CONFIRM_WORD: &str = "YES";
const POPUP_PAGE: isize = 10;
const SHORT_LIST: usize = 4;
const VIEW_PAGE: isize = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Wizard,
    Installing,
    Done,
    Failed(String),
}

#[derive(Debug)]
pub enum Action {
    None,
    Quit,
    Install(Box<Config>),
    Resolve(Vec<String>),
}

#[derive(Debug)]
pub enum Event {
    Planned(Vec<String>),
    Progress {
        index: usize,
        total: usize,
        step: String,
    },
    Finished(Result<(), String>),
    Resolved {
        key: Vec<String>,
        result: Result<Vec<Package>, String>,
    },
}

/// Расчёт списка пакетов; `key` — аргументы `dnf`, по которым он сделан.
#[derive(Debug, Clone, Default)]
pub enum Resolution {
    #[default]
    Idle,
    Blocked(String),
    Running(Vec<String>),
    Ready {
        key: Vec<String>,
        packages: Vec<Package>,
    },
    Failed {
        key: Vec<String>,
        error: String,
    },
}

#[derive(Debug, Default)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub current: String,
    pub steps: Vec<String>,
}

pub struct Popup {
    pub id: Id,
    pub options: Vec<String>,
    pub filter: Input,
    pub selected: usize,
}

impl Popup {
    fn new(id: Id, options: Vec<String>, current: Option<usize>) -> Self {
        Self {
            id,
            options,
            filter: Input::default(),
            selected: current.unwrap_or(0),
        }
    }

    /// Номера вариантов, подходящих под введённый поиск.
    pub fn matches(&self) -> Vec<usize> {
        let needle = self.filter.text().to_lowercase();

        self.options
            .iter()
            .enumerate()
            .filter(|(_, option)| option.to_lowercase().contains(&needle))
            .map(|(index, _)| index)
            .collect()
    }

    fn shift(&mut self, delta: isize) {
        let last = self.matches().len().saturating_sub(1);

        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    fn refilter(&mut self) {
        self.selected = 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Menu,
    Fields,
}

pub struct App {
    pub draft: Draft,
    pub page: usize,
    pub focus: usize,
    pub pane: Pane,
    /// Текст поля до начала правки; `Some` — поле сейчас редактируется.
    pub editing: Option<String>,
    pub visited: [bool; Page::ALL.len()],
    pub screen: Screen,
    pub notice: Option<String>,
    pub popup: Option<Popup>,
    pub resolution: Resolution,
    pub package_scroll: usize,
    pub progress: Progress,
    pub editor: Option<Editor>,
}

impl App {
    pub fn new(draft: Draft) -> Self {
        Self {
            draft,
            page: 0,
            focus: 0,
            pane: Pane::Fields,
            editing: None,
            visited: [false; Page::ALL.len()],
            screen: Screen::Wizard,
            notice: None,
            popup: None,
            resolution: Resolution::Idle,
            package_scroll: 0,
            progress: Progress::default(),
            editor: None,
        }
    }

    pub fn current_page(&self) -> Page {
        Page::ALL[self.page]
    }

    pub fn ids(&self) -> Vec<Id> {
        self.draft.visible_fields(self.current_page())
    }

    pub fn focused(&self) -> Id {
        let ids = self.ids();

        ids[self.focus.min(ids.len() - 1)]
    }

    pub fn handle(&mut self, key: KeyEvent) -> Action {
        if key.kind == KeyEventKind::Release {
            return Action::None;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        let action = match self.screen {
            Screen::Wizard if self.editor.is_some() => self.handle_editor(key, ctrl),
            Screen::Wizard if self.popup.is_some() => self.handle_popup(key),
            Screen::Wizard => match self.handle_wizard(key, ctrl) {
                Action::None => self.auto_resolve(),
                action => action,
            },
            Screen::Installing => {
                if ctrl && key.code == KeyCode::Char('c') {
                    self.notice = Some(
                        tt!(
                            "Installing: it cannot be interrupted",
                            "Идёт установка: прерывать её нельзя"
                        )
                        .into(),
                    );
                }

                Action::None
            }
            Screen::Done | Screen::Failed(_) => match key.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => Action::Quit,
                _ => Action::None,
            },
        };

        if self.screen == Screen::Wizard {
            self.draft.sync();
        }

        action
    }

    pub fn receive(&mut self, event: Event) {
        match event {
            Event::Planned(steps) => {
                self.progress.total = steps.len();
                self.progress.steps = steps;
            }
            Event::Progress { index, total, step } => {
                self.progress.done = index;
                self.progress.total = total;
                self.progress.current = step;
            }
            Event::Finished(Ok(())) => {
                self.progress.done = self.progress.total;
                self.screen = Screen::Done;
            }
            Event::Finished(Err(error)) => self.screen = Screen::Failed(error),
            Event::Resolved { key, result } => {
                if matches!(&self.resolution, Resolution::Running(current) if *current == key) {
                    self.resolution = match result {
                        Ok(packages) => Resolution::Ready { key, packages },
                        Err(error) => Resolution::Failed { key, error },
                    };
                }
            }
        }
    }

    fn handle_wizard(&mut self, key: KeyEvent, ctrl: bool) -> Action {
        self.notice = None;

        match key.code {
            KeyCode::Char('c') if ctrl => return Action::Quit,
            KeyCode::Tab | KeyCode::PageDown => self.turn(1),
            KeyCode::BackTab | KeyCode::PageUp => self.turn(-1),
            _ if self.editing.is_some() => return self.handle_edit(key, ctrl),
            _ if self.pane == Pane::Menu => self.handle_menu(key),
            _ => return self.handle_fields(key),
        }

        Action::None
    }

    fn turn(&mut self, delta: isize) {
        let target = self
            .page
            .saturating_add_signed(delta)
            .min(Page::ALL.len() - 1);

        if target != self.page {
            self.visited[self.page] = true;
            self.page = target;
            self.focus = 0;
            self.editing = None;
        }
    }

    fn handle_menu(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.turn(-1),
            KeyCode::Down => self.turn(1),
            KeyCode::Enter | KeyCode::Right => self.pane = Pane::Fields,
            _ => {}
        }
    }

    fn handle_fields(&mut self, key: KeyEvent) -> Action {
        let id = self.focused();
        let kind = self.draft.field(id).kind;

        if kind == Kind::Viewer {
            self.handle_viewer(key);
            return Action::None;
        }
        let editable = matches!(kind, Kind::Text | Kind::Secret | Kind::List);

        match key.code {
            KeyCode::Up => self.shift(-1),
            KeyCode::Down => self.shift(1),
            KeyCode::Home => self.focus = 0,
            KeyCode::End => self.focus = self.ids().len() - 1,
            KeyCode::Left | KeyCode::Esc => self.pane = Pane::Menu,
            KeyCode::Enter => return self.activate(id),
            KeyCode::Char(' ') if kind == Kind::Select => self.space(id),
            KeyCode::Char('e') | KeyCode::F(2) if editable => self.begin_edit(id),
            KeyCode::Backspace | KeyCode::Delete if kind == Kind::List => {
                self.draft.field_mut(id).items.pop();
            }
            _ => {}
        }

        Action::None
    }

    fn handle_viewer(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.scroll(-1),
            KeyCode::Down => self.scroll(1),
            KeyCode::Char(' ') => self.scroll(VIEW_PAGE),
            KeyCode::Home => self.package_scroll = 0,
            KeyCode::End => self.scroll(isize::MAX),
            KeyCode::Enter => self.resolution = Resolution::Idle,
            KeyCode::Left | KeyCode::Esc => self.pane = Pane::Menu,
            _ => {}
        }
    }

    fn scroll(&mut self, delta: isize) {
        let rows = match &self.resolution {
            Resolution::Ready { packages, .. } => packages.len(),
            _ => 0,
        };

        self.package_scroll = self
            .package_scroll
            .saturating_add_signed(delta)
            .min(rows.saturating_sub(1));
    }

    /// Аргументы `dnf` для расчёта по текущим ответам; ошибка — ответы ещё неверны.
    pub fn resolve_key(&self) -> Result<Vec<String>, String> {
        let config = self.draft.to_config()?;

        Ok(plan::dnf_args(&config, resolve::PREVIEW_ROOT, "--assumeno"))
    }

    pub fn package_count(&self) -> Option<usize> {
        match (&self.resolution, self.resolve_key()) {
            (Resolution::Ready { key, packages }, Ok(current)) if *key == current => {
                Some(packages.len())
            }
            _ => None,
        }
    }

    /// На странице «Пакеты» сама запускает расчёт, если его нет или он устарел.
    fn auto_resolve(&mut self) -> Action {
        if self.current_page() != Page::Packages {
            return Action::None;
        }

        match self.resolve_key() {
            Err(error) => self.resolution = Resolution::Blocked(error),
            Ok(key) => {
                let current = match &self.resolution {
                    Resolution::Running(known)
                    | Resolution::Ready { key: known, .. }
                    | Resolution::Failed { key: known, .. } => *known == key,
                    _ => false,
                };

                if !current {
                    self.resolution = Resolution::Running(key.clone());
                    self.package_scroll = 0;
                    return Action::Resolve(key);
                }
            }
        }

        Action::None
    }

    fn shift(&mut self, delta: isize) {
        let last = self.ids().len() - 1;

        self.focus = self.focus.saturating_add_signed(delta).min(last);
    }

    fn activate(&mut self, id: Id) -> Action {
        let field = self.draft.field(id);

        match field.kind {
            Kind::Select | Kind::Multi => self.open(id),
            Kind::Layout => self.open_editor(),
            Kind::Text | Kind::List if !field.suggestions.is_empty() => self.open(id),
            _ if id == Id::Confirm && field.value() == CONFIRM_WORD => return self.submit(),
            _ => self.begin_edit(id),
        }

        Action::None
    }

    /// Короткие списки (Да/Нет, прошивка) переключаются пробелом без окна выбора.
    fn space(&mut self, id: Id) {
        let field = self.draft.field_mut(id);

        if field.options.len() <= SHORT_LIST {
            field.index = (field.index + 1) % field.options.len();
            self.anchor(id);
        } else {
            self.open(id);
        }
    }

    /// Выбор может скрыть или показать соседние поля: фокус остаётся на том же поле.
    fn anchor(&mut self, id: Id) {
        let ids = self.ids();

        self.focus = ids
            .iter()
            .position(|candidate| *candidate == id)
            .unwrap_or_else(|| self.focus.min(ids.len() - 1));
    }

    fn open_editor(&mut self) {
        let layout = self.draft.layout().unwrap_or_default();
        let disks = self
            .draft
            .disks
            .iter()
            .map(|disk| disk.path.clone())
            .collect();

        self.editor = Some(Editor::new(layout, disks));
    }

    fn handle_editor(&mut self, key: KeyEvent, ctrl: bool) -> Action {
        let Some(editor) = self.editor.as_mut() else {
            return Action::None;
        };

        if ctrl && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }

        let outcome = editor.key(key.code);

        if editor.take_changed() {
            self.draft.set_layout(editor.layout());
        }

        if outcome == Outcome::Close {
            self.editor = None;
        }

        Action::None
    }

    fn begin_edit(&mut self, id: Id) {
        let field = self.draft.field_mut(id);

        field.input.end();
        self.editing = Some(field.value().to_string());
    }

    fn handle_edit(&mut self, key: KeyEvent, ctrl: bool) -> Action {
        let id = self.focused();

        match key.code {
            KeyCode::Enter => return self.finish_edit(id),
            KeyCode::Esc => {
                let original = self.editing.take().unwrap_or_default();
                let field = self.draft.field_mut(id);

                field.input = Input::with(&original);
            }
            KeyCode::Up | KeyCode::Down => {
                self.editing = None;
                self.shift(if key.code == KeyCode::Up { -1 } else { 1 });
            }
            code => self.type_into(id, code, ctrl),
        }

        Action::None
    }

    fn finish_edit(&mut self, id: Id) -> Action {
        let field = self.draft.field(id);

        if field.kind == Kind::List && !field.input.is_empty() {
            self.add_item(id);
            return Action::None;
        }

        self.editing = None;

        if id == Id::Confirm {
            return self.submit();
        }

        Action::None
    }

    fn open(&mut self, id: Id) {
        let field = self.draft.field(id);
        let popup = if field.kind == Kind::Multi {
            Popup::new(id, field.options.clone(), Some(0))
        } else if field.kind == Kind::Select {
            Popup::new(id, field.options.clone(), Some(field.index))
        } else {
            Popup::new(id, field.suggestions.clone(), None)
        };

        self.popup = Some(popup);
    }

    fn add_item(&mut self, id: Id) {
        let field = self.draft.field_mut(id);
        let text = field.value().trim().to_string();

        match Draft::check_item(id, &text) {
            Ok(()) => {
                if !field.items.contains(&text) {
                    field.items.push(text);
                }

                field.input.clear();
            }
            Err(error) => self.notice = Some(error),
        }
    }

    fn handle_popup(&mut self, key: KeyEvent) -> Action {
        let Some(popup) = self.popup.as_mut() else {
            return Action::None;
        };

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Action::Quit;
            }
            KeyCode::Esc => self.popup = None,
            KeyCode::Enter => self.choose(),
            KeyCode::Up => popup.shift(-1),
            KeyCode::Down => popup.shift(1),
            KeyCode::PageUp => popup.shift(-POPUP_PAGE),
            KeyCode::PageDown => popup.shift(POPUP_PAGE),
            KeyCode::Home => popup.selected = 0,
            KeyCode::End => popup.selected = popup.matches().len().saturating_sub(1),
            KeyCode::Backspace => {
                popup.filter.backspace();
                popup.refilter();
            }
            KeyCode::Char(symbol) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                popup.filter.insert(symbol);
                popup.refilter();
            }
            _ => {}
        }

        Action::None
    }

    /// Выбранный вариант; в полях с подсказками без совпадений берётся набранный текст.
    fn choose(&mut self) {
        if self
            .popup
            .as_ref()
            .is_some_and(|popup| self.draft.field(popup.id).kind == Kind::Multi)
        {
            return self.toggle();
        }

        let Some(popup) = self.popup.take() else {
            return;
        };
        let kind = self.draft.field(popup.id).kind;
        let typed = popup.filter.text().trim().to_string();
        let picked = popup.matches().get(popup.selected).copied();

        let value = match picked {
            Some(index) => popup.options[index].clone(),
            None if kind != Kind::Select && !typed.is_empty() => typed,
            None => {
                self.popup = Some(popup);
                return;
            }
        };

        if kind == Kind::List
            && let Err(error) = Draft::check_item(popup.id, &value)
        {
            self.notice = Some(error);
            self.popup = Some(popup);
            return;
        }

        let field = self.draft.field_mut(popup.id);

        match (kind, picked) {
            (Kind::Select, Some(index)) => field.index = index,
            (Kind::Text, _) => field.input = Input::with(&value),
            (Kind::List, _) if !field.items.contains(&value) => field.items.push(value),
            _ => {}
        }

        self.anchor(popup.id);

        if popup.id == Id::Scheme && self.draft.is_custom() {
            self.open_editor();
        }
    }

    fn toggle(&mut self) {
        let Some(popup) = self.popup.as_ref() else {
            return;
        };
        let Some(&index) = popup.matches().get(popup.selected) else {
            return;
        };
        let field = self.draft.field_mut(popup.id);

        field.checked[index] = !field.checked[index];
    }

    fn submit(&mut self) -> Action {
        if self.draft.field(Id::Confirm).value() != CONFIRM_WORD {
            self.notice = Some(tr!(
                "Type {CONFIRM_WORD} to start",
                "Для запуска введите {CONFIRM_WORD}"
            ));
            return Action::None;
        }

        match self.draft.to_config() {
            Ok(config) => {
                self.screen = Screen::Installing;
                self.notice = None;
                Action::Install(Box::new(config))
            }
            Err(error) => {
                self.jump_to_error();
                self.notice = Some(error);
                Action::None
            }
        }
    }

    fn jump_to_error(&mut self) {
        let Some((page, id, _)) = self.draft.first_error() else {
            return;
        };

        self.visited[self.page] = true;
        self.pane = Pane::Fields;
        self.editing = None;
        self.page = Page::ALL
            .iter()
            .position(|candidate| *candidate == page)
            .unwrap_or(0);
        self.focus = self
            .ids()
            .iter()
            .position(|candidate| *candidate == id)
            .unwrap_or(0);
    }

    fn type_into(&mut self, id: Id, code: KeyCode, ctrl: bool) {
        let field = self.draft.field_mut(id);

        match code {
            KeyCode::Char(symbol) if !ctrl => field.input.insert(symbol),
            KeyCode::Backspace if field.kind == Kind::List && field.input.is_empty() => {
                self.draft.field_mut(id).items.pop();
            }
            KeyCode::Backspace => field.input.backspace(),
            KeyCode::Delete => field.input.delete(),
            KeyCode::Left => field.input.left(),
            KeyCode::Right => field.input.right(),
            KeyCode::Home => field.input.home(),
            KeyCode::End => field.input.end(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyEventState;

    use super::*;
    use crate::draft::tests::filled;
    use crate::sources::Sources;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn ctrl(symbol: char) -> KeyEvent {
        KeyEvent {
            modifiers: KeyModifiers::CONTROL,
            ..key(KeyCode::Char(symbol))
        }
    }

    fn app() -> App {
        crate::testing::app_on_disk_page(filled())
    }

    fn press(app: &mut App, codes: &[KeyCode]) {
        for code in codes {
            app.handle(key(*code));
        }
    }

    fn type_text(app: &mut App, text: &str) {
        for symbol in text.chars() {
            app.handle(key(KeyCode::Char(symbol)));
        }
    }

    fn go_to(app: &mut App, page: Page, id: Id) {
        app.page = Page::ALL
            .iter()
            .position(|candidate| *candidate == page)
            .unwrap();
        app.focus = app
            .ids()
            .iter()
            .position(|candidate| *candidate == id)
            .unwrap();
        app.pane = Pane::Fields;
        app.editing = None;
    }

    fn with_sources(sources: Sources) -> App {
        crate::testing::app_on_disk_page(Draft::new(sources))
    }

    #[test]
    fn the_menu_has_its_own_focus() {
        let mut app = app();

        assert_eq!(app.pane, Pane::Fields);

        press(&mut app, &[KeyCode::Left]);
        assert_eq!(app.pane, Pane::Menu);

        press(&mut app, &[KeyCode::Down, KeyCode::Down]);
        assert_eq!(app.current_page(), Page::System);
        assert!(app.visited[1] && app.visited[2]);

        press(&mut app, &[KeyCode::Up]);
        assert_eq!(app.current_page(), Page::Repos);

        press(&mut app, &[KeyCode::Enter]);
        assert_eq!((app.pane, app.focused()), (Pane::Fields, Id::Repos));
    }

    #[test]
    fn tab_and_page_keys_switch_pages_from_anywhere() {
        let mut app = app();

        press(&mut app, &[KeyCode::Tab]);
        assert_eq!((app.current_page(), app.pane), (Page::Repos, Pane::Fields));

        press(&mut app, &[KeyCode::PageDown, KeyCode::BackTab]);
        assert_eq!(app.current_page(), Page::Repos);

        press(&mut app, &[KeyCode::PageUp, KeyCode::PageUp]);
        assert_eq!(app.page, 0);

        app.page = Page::ALL.len() - 1;
        press(&mut app, &[KeyCode::Tab]);
        assert_eq!(app.current_page(), Page::Summary);
    }

    #[test]
    fn arrows_move_between_fields_and_stop_at_the_edges() {
        let mut app = app();

        press(&mut app, &[KeyCode::Up]);
        assert_eq!(app.focused(), Id::Scheme);

        press(&mut app, &[KeyCode::Down; 6]);
        assert_eq!(
            (app.current_page(), app.focused()),
            (Page::Disk, Id::Firmware)
        );

        press(&mut app, &[KeyCode::Home]);
        assert_eq!(app.focused(), Id::Scheme);
    }

    #[test]
    fn enter_edits_a_text_field_and_does_not_jump_away() {
        let mut app = app();

        go_to(&mut app, Page::System, Id::Hostname);
        press(&mut app, &[KeyCode::Enter]);
        assert!(app.editing.is_some());

        type_text(&mut app, "-1");
        press(&mut app, &[KeyCode::Backspace]);
        type_text(&mut app, "2");
        press(&mut app, &[KeyCode::Enter]);

        assert!(app.editing.is_none());
        assert_eq!(app.focused(), Id::Hostname);
        assert_eq!(app.draft.field(Id::Hostname).value(), "moonshine-2");
    }

    #[test]
    fn typing_outside_the_edit_mode_changes_nothing() {
        let mut app = app();

        go_to(&mut app, Page::System, Id::Hostname);
        type_text(&mut app, "zzz");

        assert_eq!(app.draft.field(Id::Hostname).value(), "moonshine");
    }

    #[test]
    fn escape_restores_the_text_from_before_the_edit() {
        let mut app = app();

        go_to(&mut app, Page::System, Id::Hostname);
        press(&mut app, &[KeyCode::Enter]);
        type_text(&mut app, "xyz");
        press(&mut app, &[KeyCode::Esc]);

        assert!(app.editing.is_none() && app.pane == Pane::Fields);
        assert_eq!(app.draft.field(Id::Hostname).value(), "moonshine");
    }

    #[test]
    fn the_e_key_edits_a_field_that_also_has_suggestions() {
        let mut app = with_sources(Sources {
            interfaces: vec!["eth0".into()],
            ..Sources::default()
        });

        go_to(&mut app, Page::SystemNetwork, Id::SystemInterface);
        press(&mut app, &[KeyCode::Char('e')]);

        assert!(app.editing.is_some() && app.popup.is_none());

        type_text(&mut app, "2");
        press(&mut app, &[KeyCode::Enter]);

        assert_eq!(app.draft.field(Id::SystemInterface).value(), "2");
    }

    #[test]
    fn arrows_while_editing_commit_the_text_and_move_on() {
        let mut app = app();

        go_to(&mut app, Page::System, Id::Hostname);
        press(&mut app, &[KeyCode::Enter]);
        type_text(&mut app, "x");
        press(&mut app, &[KeyCode::Down]);

        assert!(app.editing.is_none() && app.focused() == Id::User);
        assert_eq!(app.draft.field(Id::Hostname).value(), "moonshinex");
    }

    #[test]
    fn enter_opens_a_list_and_the_choice_keeps_the_focus() {
        let mut app = app();

        go_to(&mut app, Page::Disk, Id::Disk);
        press(&mut app, &[KeyCode::Enter]);
        assert!(app.popup.is_some());

        press(&mut app, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);

        assert!(app.popup.is_none());
        assert_eq!(app.draft.field(Id::Disk).index, 1);
        assert_eq!(app.focused(), Id::Disk);
    }

    #[test]
    fn repositories_are_switched_with_enter_inside_an_open_list() {
        use crate::repos::Repo;

        let repo = |id: &str, enabled| Repo {
            id: id.into(),
            name: id.into(),
            enabled,
            file: format!("/etc/yum.repos.d/{id}.repo"),
        };
        let mut app = with_sources(Sources {
            repos: vec![repo("baseos", true), repo("debug", false)],
            ..Sources::default()
        });

        go_to(&mut app, Page::Repos, Id::Repos);
        press(&mut app, &[KeyCode::Enter]);
        assert!(app.popup.is_some());

        press(&mut app, &[KeyCode::Enter, KeyCode::Down, KeyCode::Enter]);

        assert!(app.popup.is_some(), "список остаётся открытым");
        assert_eq!(app.draft.field(Id::Repos).checked, [false, true]);

        press(&mut app, &[KeyCode::Esc]);

        assert!(app.popup.is_none());
        assert_eq!(app.draft.field(Id::Repos).chosen(), ["debug"]);
    }

    #[test]
    fn a_filtered_repository_list_toggles_the_matching_item() {
        use crate::repos::Repo;

        let repo = |id: &str| Repo {
            id: id.into(),
            name: id.into(),
            enabled: false,
            file: "f.repo".into(),
        };
        let mut app = with_sources(Sources {
            repos: vec![repo("baseos"), repo("moon")],
            ..Sources::default()
        });

        go_to(&mut app, Page::Repos, Id::Repos);
        press(&mut app, &[KeyCode::Enter]);
        type_text(&mut app, "moon");
        press(&mut app, &[KeyCode::Enter]);

        assert_eq!(app.draft.field(Id::Repos).checked, [false, true]);
    }

    fn repo_app() -> App {
        use crate::repos::Repo;

        let mut app = with_sources(Sources {
            disks: vec![crate::draft::tests::disk("/dev/sda")],
            repos: vec![Repo {
                id: "baseos".into(),
                name: "Base".into(),
                enabled: true,
                file: "/etc/yum.repos.d/a.repo".into(),
            }],
            ..Sources::default()
        });

        for (id, value) in [
            (Id::User, "alice"),
            (Id::Password, "pw"),
            (Id::PasswordAgain, "pw"),
            (Id::Passphrase, "phrase"),
            (Id::PassphraseAgain, "phrase"),
        ] {
            app.draft.field_mut(id).input = Input::with(value);
        }

        app
    }

    fn package(name: &str) -> Package {
        Package {
            name: name.into(),
            arch: "x86_64".into(),
            version: "1-1".into(),
            repo: "baseos".into(),
        }
    }

    #[test]
    fn opening_the_packages_page_starts_the_calculation_once() {
        let mut app = repo_app();

        go_to(&mut app, Page::SystemNetwork, Id::SystemMode);

        let Action::Resolve(args) = app.handle(key(KeyCode::Tab)) else {
            panic!("ожидался расчёт");
        };

        assert_eq!(app.current_page(), Page::Packages);
        assert!(
            args.contains(&"--assumeno".to_string())
                && args.contains(&"--enablerepo=baseos".to_string())
        );
        assert!(args[0].starts_with("--installroot=") && !args[0].contains("/mnt/target"));
        assert!(matches!(app.resolution, Resolution::Running(_)));

        assert!(
            matches!(app.handle(key(KeyCode::Down)), Action::None),
            "уже считается"
        );
    }

    #[test]
    fn the_result_is_kept_and_counts_while_fresh_and_goes_stale_with_the_answers() {
        let mut app = repo_app();

        go_to(&mut app, Page::SystemNetwork, Id::SystemMode);

        let Action::Resolve(key_args) = app.handle(key(KeyCode::Tab)) else {
            panic!("ожидался расчёт");
        };

        app.receive(Event::Resolved {
            key: key_args.clone(),
            result: Ok(vec![package("zsh"), package("bash")]),
        });

        assert_eq!(app.package_count(), Some(2));
        assert!(matches!(app.handle(key(KeyCode::BackTab)), Action::None));

        let Action::None = app.handle(key(KeyCode::Tab)) else {
            panic!("расчёт актуален, повторять не нужно");
        };

        app.draft.field_mut(Id::Packages).input = Input::with("mc");

        assert_eq!(app.package_count(), None);
        assert!(matches!(app.handle(key(KeyCode::Down)), Action::Resolve(new) if new != key_args));
    }

    #[test]
    fn a_stale_answer_is_ignored() {
        let mut app = repo_app();

        app.resolution = Resolution::Running(vec!["new".into()]);
        app.receive(Event::Resolved {
            key: vec!["old".into()],
            result: Ok(vec![package("a")]),
        });

        assert!(matches!(app.resolution, Resolution::Running(_)));
    }

    #[test]
    fn enter_on_the_list_recalculates_and_a_failure_is_shown() {
        let mut app = repo_app();

        go_to(&mut app, Page::Packages, Id::PackageList);

        let Action::Resolve(args) = app.handle(key(KeyCode::Enter)) else {
            panic!("ожидался расчёт");
        };

        app.receive(Event::Resolved {
            key: args,
            result: Err("нет сети".into()),
        });

        assert!(matches!(&app.resolution, Resolution::Failed { error, .. } if error == "нет сети"));
        assert!(
            matches!(app.handle(key(KeyCode::Down)), Action::None),
            "ошибка не зацикливает расчёт"
        );
        assert!(
            matches!(app.handle(key(KeyCode::Enter)), Action::Resolve(_)),
            "Enter повторяет"
        );
    }

    #[test]
    fn an_incomplete_form_blocks_the_calculation_with_the_reason() {
        let mut app = repo_app();

        app.draft.field_mut(Id::User).input = Input::with("");
        go_to(&mut app, Page::SystemNetwork, Id::SystemMode);

        assert!(matches!(app.handle(key(KeyCode::Tab)), Action::None));
        assert!(matches!(app.resolution, Resolution::Blocked(_)));
    }

    #[test]
    fn the_package_list_scrolls_inside_its_bounds() {
        let mut app = repo_app();

        go_to(&mut app, Page::Packages, Id::PackageList);
        app.resolution = Resolution::Ready {
            key: app.resolve_key().unwrap(),
            packages: (0..30)
                .map(|number| package(&format!("p{number}")))
                .collect(),
        };

        press(&mut app, &[KeyCode::Up]);
        assert_eq!(app.package_scroll, 0);

        press(&mut app, &[KeyCode::Down, KeyCode::Down]);
        assert_eq!(app.package_scroll, 2);

        press(
            &mut app,
            &[KeyCode::Char(' '), KeyCode::Char(' '), KeyCode::Char(' ')],
        );
        assert_eq!(app.package_scroll, 29);

        press(&mut app, &[KeyCode::Home]);
        assert_eq!(app.package_scroll, 0);

        press(&mut app, &[KeyCode::End]);
        assert_eq!(app.package_scroll, 29);
    }

    #[test]
    fn escape_leaves_the_value_untouched() {
        let mut app = app();

        press(&mut app, &[KeyCode::Enter, KeyCode::Down, KeyCode::Esc]);

        assert!(app.popup.is_none() && app.draft.field(Id::Disk).index == 0);
        assert_eq!(app.pane, Pane::Fields);
    }

    #[test]
    fn space_flips_a_short_list_and_opens_a_long_one() {
        let mut app = with_sources(Sources {
            timezones: vec![
                "A/B".into(),
                "C/D".into(),
                "E/F".into(),
                "G/H".into(),
                "UTC".into(),
            ],
            ..Sources::default()
        });

        go_to(&mut app, Page::Disk, Id::Firmware);
        press(&mut app, &[KeyCode::Char(' ')]);
        assert_eq!(app.draft.field(Id::Firmware).choice_text(), "U-Boot");
        press(&mut app, &[KeyCode::Char(' ')]);
        assert_eq!(app.draft.field(Id::Firmware).choice_text(), "BIOS+UEFI");
        press(&mut app, &[KeyCode::Char(' ')]);
        assert_eq!(app.draft.field(Id::Firmware).choice_text(), "UEFI");
        press(&mut app, &[KeyCode::Char(' ')]);
        assert_eq!(app.draft.field(Id::Firmware).choice_text(), "BIOS");
        assert!(app.popup.is_none());

        go_to(&mut app, Page::System, Id::Timezone);
        press(&mut app, &[KeyCode::Char(' ')]);
        assert!(app.popup.is_some());
    }

    #[test]
    fn typing_in_a_list_filters_it() {
        let mut app = with_sources(Sources {
            timezones: vec![
                "Asia/Tokyo".into(),
                "Europe/Moscow".into(),
                "Europe/Paris".into(),
                "UTC".into(),
            ],
            ..Sources::default()
        });

        go_to(&mut app, Page::System, Id::Timezone);
        press(&mut app, &[KeyCode::Enter]);
        type_text(&mut app, "PAR");

        assert_eq!(app.popup.as_ref().unwrap().matches(), [2]);

        press(&mut app, &[KeyCode::Enter]);

        assert_eq!(app.draft.field(Id::Timezone).choice_text(), "Europe/Paris");

        press(&mut app, &[KeyCode::Enter]);
        type_text(&mut app, "zzz");
        press(&mut app, &[KeyCode::Enter]);

        assert!(
            app.popup.is_some(),
            "в списке без совпадений выбирать нечего"
        );

        press(&mut app, &[KeyCode::Backspace; 3]);
        type_text(&mut app, "a");

        assert_eq!(app.popup.as_ref().unwrap().filter.text(), "a");
    }

    #[test]
    fn a_text_field_takes_a_suggestion_or_a_typed_value() {
        let mut app = with_sources(Sources {
            interfaces: vec!["enp3s0".into(), "eth0".into()],
            ..Sources::default()
        });

        app.draft.field_mut(Id::SystemMode).index = 1;
        go_to(&mut app, Page::SystemNetwork, Id::SystemInterface);
        press(&mut app, &[KeyCode::Enter, KeyCode::Down, KeyCode::Enter]);

        assert_eq!(app.draft.field(Id::SystemInterface).value(), "eth0");

        press(&mut app, &[KeyCode::Enter]);
        type_text(&mut app, "wlan9");
        press(&mut app, &[KeyCode::Enter]);

        assert_eq!(app.draft.field(Id::SystemInterface).value(), "wlan9");
    }

    #[test]
    fn a_list_field_collects_suggestions_without_duplicates() {
        let mut app = app();

        app.draft.field_mut(Id::SshKeys).suggestions =
            vec!["ssh-ed25519 AAAA a".into(), "ssh-ed25519 AAAA b".into()];
        go_to(&mut app, Page::Encryption, Id::SshKeys);
        press(&mut app, &[KeyCode::Enter, KeyCode::Enter]);
        press(&mut app, &[KeyCode::Enter, KeyCode::Enter]);
        press(&mut app, &[KeyCode::Enter, KeyCode::Down, KeyCode::Enter]);

        assert_eq!(
            app.draft.field(Id::SshKeys).items,
            ["ssh-ed25519 AAAA a", "ssh-ed25519 AAAA b"]
        );
    }

    #[test]
    fn list_items_are_typed_added_one_by_one_and_removed_in_browse_mode() {
        let mut app = app();

        go_to(&mut app, Page::Encryption, Id::SshKeys);
        press(&mut app, &[KeyCode::Enter]);
        type_text(&mut app, "ssh-ed25519 AAAA one");
        press(&mut app, &[KeyCode::Enter]);

        assert!(
            app.editing.is_some(),
            "после добавления можно вводить следующий"
        );

        type_text(&mut app, "ssh-ed25519 BBBB two");
        press(&mut app, &[KeyCode::Enter, KeyCode::Enter]);

        assert!(app.editing.is_none());
        assert_eq!(app.draft.field(Id::SshKeys).items.len(), 2);

        press(&mut app, &[KeyCode::Backspace]);

        assert_eq!(app.draft.field(Id::SshKeys).items, ["ssh-ed25519 AAAA one"]);
    }

    #[test]
    fn a_bad_list_item_is_refused_with_a_notice() {
        let mut app = app();

        go_to(&mut app, Page::Encryption, Id::SshKeys);
        press(&mut app, &[KeyCode::Enter]);
        type_text(&mut app, "   ");
        press(&mut app, &[KeyCode::Enter]);

        assert!(app.draft.field(Id::SshKeys).items.is_empty() && app.notice.is_some());
        assert!(app.editing.is_some());
    }

    #[test]
    fn switching_encryption_off_hides_its_fields() {
        let mut app = app();

        go_to(&mut app, Page::Encryption, Id::Encrypt);
        assert_eq!(app.ids().len(), 6);

        press(&mut app, &[KeyCode::Char(' ')]);
        assert_eq!(app.ids(), [Id::Encrypt]);

        press(&mut app, &[KeyCode::Char(' ')]);
        assert_eq!(app.ids().len(), 6);
    }

    #[test]
    fn installation_needs_the_confirmation_word() {
        let mut app = app();

        go_to(&mut app, Page::Summary, Id::Confirm);
        press(&mut app, &[KeyCode::Enter]);
        type_text(&mut app, "yes");

        assert!(matches!(app.handle(key(KeyCode::Enter)), Action::None));
        assert_eq!(app.screen, Screen::Wizard);
        assert!(app.notice.is_some());

        app.draft.field_mut(Id::Confirm).input = Input::with("YES");

        assert!(matches!(
            app.handle(key(KeyCode::Enter)),
            Action::Install(_)
        ));
        assert_eq!(app.screen, Screen::Installing);
    }

    #[test]
    fn typing_the_word_and_pressing_enter_starts_the_installation() {
        let mut app = app();

        go_to(&mut app, Page::Summary, Id::Confirm);
        press(&mut app, &[KeyCode::Enter]);
        type_text(&mut app, "YES");

        assert!(matches!(
            app.handle(key(KeyCode::Enter)),
            Action::Install(_)
        ));
    }

    #[test]
    fn a_mistake_sends_the_user_back_to_the_broken_field() {
        let mut app = app();

        app.draft.field_mut(Id::Hostname).input = Input::with("-bad");
        go_to(&mut app, Page::Summary, Id::Confirm);
        app.pane = Pane::Menu;
        app.draft.field_mut(Id::Confirm).input = Input::with("YES");
        app.pane = Pane::Fields;

        assert!(matches!(app.handle(key(KeyCode::Enter)), Action::None));
        assert_eq!(
            (app.current_page(), app.focused(), app.pane),
            (Page::System, Id::Hostname, Pane::Fields)
        );
        assert!(app.notice.is_some() && app.visited[Page::ALL.len() - 1]);
    }

    #[test]
    fn ctrl_c_quits_only_outside_the_installation() {
        let mut app = app();

        assert!(matches!(app.handle(ctrl('c')), Action::Quit));

        app.screen = Screen::Installing;

        assert!(matches!(app.handle(ctrl('c')), Action::None));
        assert!(app.notice.is_some());
    }

    #[test]
    fn progress_events_end_in_done_or_failed() {
        let mut app = app();

        app.screen = Screen::Installing;
        app.receive(Event::Progress {
            index: 2,
            total: 10,
            step: "sfdisk".into(),
        });

        assert_eq!(
            (
                app.progress.done,
                app.progress.total,
                app.progress.current.as_str()
            ),
            (2, 10, "sfdisk")
        );

        app.receive(Event::Finished(Ok(())));
        assert_eq!((app.screen.clone(), app.progress.done), (Screen::Done, 10));
        assert!(matches!(app.handle(key(KeyCode::Enter)), Action::Quit));

        let mut app = App::new(filled());

        app.receive(Event::Finished(Err("boom".into())));
        assert_eq!(app.screen, Screen::Failed("boom".into()));
    }

    #[test]
    fn enter_on_the_layout_field_opens_the_editor_and_escape_closes_it() {
        let mut app = app();

        go_to(&mut app, Page::Disk, Id::Layout);
        press(&mut app, &[KeyCode::Enter]);

        assert!(app.editor.is_some());

        press(&mut app, &[KeyCode::Esc]);

        assert!(app.editor.is_none());
        assert_eq!(app.draft.scheme(), 0);
        assert_eq!(app.focused(), Id::Layout);
    }

    #[test]
    fn a_change_in_the_editor_turns_the_scheme_into_a_custom_layout() {
        let mut app = app();

        go_to(&mut app, Page::Disk, Id::Layout);
        press(
            &mut app,
            &[KeyCode::Enter, KeyCode::Down, KeyCode::Char('n')],
        );
        type_text(&mut app, "swap");
        press(&mut app, &[KeyCode::Enter, KeyCode::Esc]);

        let items = &app.draft.field(Id::Layout).items;

        assert_eq!(app.draft.scheme(), 3);
        assert_eq!(items.len(), 3);
        assert!(items[0].contains("point=boot") && items[1].contains("point=swap"));
    }

    #[test]
    fn a_broken_layout_is_reported_while_the_editor_is_open() {
        let mut app = app();

        go_to(&mut app, Page::Disk, Id::Layout);
        press(&mut app, &[KeyCode::Enter, KeyCode::Down, KeyCode::Down]);
        press(&mut app, &[KeyCode::Char('d')]);

        assert!(app.editor.is_some());
        assert!(app.draft.layout_problem().is_some());

        press(&mut app, &[KeyCode::Char('u')]);

        assert!(app.draft.layout_problem().is_none());
    }

    #[test]
    fn keys_inside_the_editor_do_not_reach_the_wizard() {
        let mut app = app();

        go_to(&mut app, Page::Disk, Id::Layout);
        press(&mut app, &[KeyCode::Enter, KeyCode::Tab, KeyCode::PageDown]);

        assert_eq!(app.current_page(), Page::Disk);
        assert!(matches!(app.handle(ctrl('c')), Action::Quit));
    }

    #[test]
    fn choosing_another_scheme_rewrites_the_rows() {
        let mut app = app();

        go_to(&mut app, Page::Disk, Id::Scheme);
        press(&mut app, &[KeyCode::Enter, KeyCode::Down, KeyCode::Enter]);

        assert_eq!(app.draft.scheme(), 1);
        assert!(
            app.draft
                .field(Id::Layout)
                .items
                .iter()
                .any(|row| row.contains("point=lvm:moonshine"))
        );
    }

    #[test]
    fn choosing_the_custom_scheme_opens_the_editor_with_the_previous_layout() {
        let mut app = app();

        go_to(&mut app, Page::Disk, Id::Scheme);
        press(
            &mut app,
            &[KeyCode::Enter, KeyCode::Down, KeyCode::Down, KeyCode::Down],
        );
        press(&mut app, &[KeyCode::Enter]);

        assert_eq!(app.draft.scheme(), 3);

        let editor = app.editor.as_ref().expect("редактор должен открыться");

        assert_eq!(editor.layout().partitions.len(), 2);
    }
}
