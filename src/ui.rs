use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Clear;
use ratatui::widgets::Gauge;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;

use crate::app::App;
use crate::app::Pane;
use crate::app::Popup;
use crate::app::Resolution;
use crate::app::Screen;
use crate::draft::Draft;
use crate::draft::Field;
use crate::draft::Id;
use crate::draft::Kind;
use crate::draft::Page;
use crate::editor::Editor;
use crate::i18n::tr;
use crate::i18n::tt;

const ACCENT: Color = Color::Cyan;
const BAR: Color = Color::Rgb(48, 52, 64);
const BAR_EDIT: Color = Color::Rgb(70, 78, 110);
const SIDEBAR_WIDTH: u16 = 32;
const BAR_WIDTH: usize = 56;
const SECRET_DOT: char = '•';

fn accent() -> Style {
    Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
}

fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

fn error_style() -> Style {
    Style::new().fg(Color::Red)
}

fn block(title: &str, active: bool) -> Block<'_> {
    let (border, text) = if active {
        (accent(), accent())
    } else {
        (dim(), dim())
    };

    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Span::styled(format!(" {title} "), text))
}

pub fn draw(frame: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_header(frame, header, app);
    draw_footer(frame, footer, app);

    match &app.screen {
        Screen::Wizard => draw_wizard(frame, body, app),
        Screen::Installing => draw_installing(frame, body, app),
        Screen::Done => draw_result(
            frame,
            body,
            tt!("Done", "Готово"),
            Color::Green,
            tt!(
                "The system is installed. You can reboot.",
                "Система установлена. Можно перезагружаться."
            ),
        ),
        Screen::Failed(error) => draw_failure(frame, body, app, error),
    }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let subtitle = match app.screen {
        Screen::Wizard => tr!(
            "step {} of {} — {}",
            "шаг {} из {} — {}",
            app.page + 1,
            Page::ALL.len(),
            app.current_page().title()
        ),
        Screen::Installing => tt!("installing", "установка").into(),
        Screen::Done => tt!("done", "готово").into(),
        Screen::Failed(_) => tt!("error", "ошибка").into(),
    };
    let title = Line::from(vec![
        Span::styled("Moonshine", accent()),
        Span::raw("  ·  "),
        Span::raw(subtitle),
    ]);

    frame.render_widget(
        Paragraph::new(title).alignment(Alignment::Center).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(dim()),
        ),
        area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let line = match (&app.notice, &app.screen) {
        (Some(notice), _) => Line::from(Span::styled(format!(" ✗ {notice}"), error_style())),
        (None, Screen::Wizard) => Line::from(Span::styled(wizard_hint(app), dim())),
        (None, Screen::Installing) => Line::from(Span::styled(
            tt!(
                " Installing, do not switch the power off",
                " Идёт установка, не выключайте питание"
            ),
            dim(),
        )),
        (None, _) => Line::from(Span::styled(tt!(" Enter — exit", " Enter — выход"), dim())),
    };

    frame.render_widget(Paragraph::new(line), area);
}

fn wizard_hint(app: &App) -> &'static str {
    if let Some(editor) = &app.editor {
        return if editor.form.is_some() {
            tt!(
                " Layout editor · input line",
                " Редактор разметки · строка ввода"
            )
        } else {
            tt!(
                " Layout editor · keys are listed at the bottom of the window",
                " Редактор разметки · клавиши подсказаны внизу окна"
            )
        };
    }

    if let Some(popup) = &app.popup {
        let kind = app.draft.field(popup.id).kind;

        return if kind == Kind::Multi {
            tt!(
                " ↑↓ choose · Enter toggle · Esc done · type to search",
                " ↑↓ выбор · Enter включить/выключить · Esc готово · печатайте для поиска"
            )
        } else if kind == Kind::Select {
            tt!(
                " ↑↓ choose · Enter confirm · Esc cancel · type to search",
                " ↑↓ выбор · Enter подтвердить · Esc отмена · печатайте для поиска"
            )
        } else {
            tt!(
                " ↑↓ choose · Enter confirm · Esc cancel · own value: type it and press Enter",
                " ↑↓ выбор · Enter подтвердить · Esc отмена · свой вариант: наберите и Enter"
            )
        };
    }

    if app.editing.is_some() {
        return tt!(
            " Editing: Enter done · Esc cancel · ↑↓ another field · Tab page",
            " Правка: Enter готово · Esc отменить · ↑↓ другое поле · Tab раздел"
        );
    }

    match app.pane {
        Pane::Menu => tt!(
            " ↑↓ page · Enter/→ to fields · Tab page · Ctrl+C exit",
            " ↑↓ раздел · Enter/→ к полям · Tab раздел · Ctrl+C выход"
        ),
        Pane::Fields if app.draft.field(app.focused()).kind == Kind::Viewer => {
            tt!(
                " ↑↓ scroll · Space page · Home to top · Enter recalculate · ← menu · Tab page",
                " ↑↓ прокрутка · Пробел страница · Home — в начало · Enter пересчитать · ← меню · Tab раздел"
            )
        }
        Pane::Fields => {
            tt!(
                " ↑↓ field · Enter change · Space switch · E text · ← menu · Tab page",
                " ↑↓ поле · Enter изменить · Пробел переключить · E текст · ← меню · Tab раздел"
            )
        }
    }
}

fn draw_wizard(frame: &mut Frame, area: Rect, app: &App) {
    let [sidebar, content] =
        Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)]).areas(area);

    draw_sidebar(frame, sidebar, app);

    let page = app.current_page();
    let frame_block = block(page.title(), app.pane == Pane::Fields);
    let inner = frame_block.inner(content);
    let view = page_view(app, inner.width as usize);
    let height = inner.height as usize;
    let scroll = view.scroll.unwrap_or_else(|| {
        view.focus
            .1
            .saturating_sub(height.saturating_sub(1))
            .min(view.focus.0)
    });

    frame.render_widget(frame_block, content);
    frame.render_widget(Paragraph::new(view.lines).scroll((scroll as u16, 0)), inner);

    if let Some((column, row)) = view.cursor
        && row >= scroll
        && row - scroll < height
    {
        frame.set_cursor_position((inner.x + column as u16, inner.y + (row - scroll) as u16));
    }

    if let Some(popup) = &app.popup {
        let field = app.draft.field(popup.id);
        let checked = (field.kind == Kind::Multi).then_some(field.checked.as_slice());

        draw_popup(frame, area, popup, field.label, checked);
    }

    if let Some(editor) = &app.editor {
        draw_editor(frame, area, app, editor);
    }
}

fn draw_editor(frame: &mut Frame, area: Rect, app: &App, editor: &Editor) {
    let tree = editor.tree();
    let broken = editor.broken();
    let problem = app.draft.layout_problem();
    let form_height = if editor.form.is_some() { 4 } else { 0 };
    let frame_block = block(tt!("Layout editor", "Редактор разметки"), true);
    let inner = frame_block.inner(area);
    let keys = keybar(&editor.keys(), inner.width as usize);
    let [list, form, status, hints] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(form_height),
        Constraint::Length(2),
        Constraint::Length(keys.len() as u16),
    ])
    .areas(inner);
    let rows = list.height as usize;
    let first = editor.cursor.saturating_sub(rows.saturating_sub(1));
    let lines: Vec<Line> = tree
        .iter()
        .enumerate()
        .skip(first)
        .take(rows)
        .map(|(position, entry)| {
            let marker = match editor.folded(&entry.node) {
                Some(true) => "▸ ",
                Some(false) => "▾ ",
                None => "  ",
            };
            let text = editor.text(&entry.node, |disk| app.draft.disk_size(disk));
            let bad = broken.contains(&entry.node);
            let mut style = Style::new();

            if bad {
                style = style.fg(Color::Red);
            }

            if position == editor.cursor {
                style = style
                    .fg(Color::Black)
                    .bg(if bad { Color::Red } else { ACCENT })
                    .add_modifier(Modifier::BOLD);
            }

            Line::from(Span::styled(
                format!(" {}{marker}{text}", "   ".repeat(entry.depth)),
                style,
            ))
        })
        .collect();

    frame.render_widget(Clear, area);
    frame.render_widget(frame_block, area);

    if lines.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                tt!(
                    "The layout is empty: a — add a partition",
                    "Разметка пуста: a — добавить раздел"
                ),
                Style::new().fg(Color::White),
            )),
            list,
        );
    } else {
        frame.render_widget(Paragraph::new(lines), list);
    }

    let verdict = match problem {
        Some(error) => Line::from(Span::styled(format!("✗ {error}"), error_style())),
        None => Line::from(Span::styled(
            tt!("✓ the layout is valid", "✓ разметка корректна"),
            Style::new().fg(Color::Green),
        )),
    };

    frame.render_widget(Paragraph::new(verdict).wrap(Wrap { trim: true }), status);
    frame.render_widget(Paragraph::new(keys), hints);

    if let Some(editing) = &editor.form {
        let title = format!(" {} ", editing.item.title());
        let notes = match &editing.error {
            Some(error) => Line::from(Span::styled(format!("✗ {error}"), error_style())),
            None => Line::from(Span::styled(
                tr!("disks: {}", "диски: {}", editor.disks().join(" ")),
                Style::new().fg(Color::White),
            )),
        };
        let field_block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(accent())
            .title(Span::styled(title, accent()));
        let field = field_block.inner(form);

        frame.render_widget(field_block, Rect { height: 3, ..form });
        frame.render_widget(Paragraph::new(editing.input.text().to_string()), field);
        frame.render_widget(
            Paragraph::new(notes),
            Rect {
                y: form.y + 3,
                height: 1,
                ..form
            },
        );
        frame.set_cursor_position((field.x + editing.input.cursor() as u16, field.y));
    }
}

/// Клавиши плашками, подписи рядом; строки переносятся по ширине окна.
fn keybar(keys: &[(&str, &str)], width: usize) -> Vec<Line<'static>> {
    let chip = Style::new()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::new().fg(Color::White);
    let mut lines = Vec::new();
    let mut spans = Vec::new();
    let mut used = 0;

    for (key, label) in keys {
        let size = key.chars().count() + label.chars().count() + 5;

        if used > 0 && used + size > width {
            lines.push(Line::from(std::mem::take(&mut spans)));
            used = 0;
        }

        spans.push(Span::styled(format!(" {key} "), chip));
        spans.push(Span::styled(format!(" {label}  "), label_style));
        used += size;
    }

    lines.push(Line::from(spans));
    lines
}

fn draw_popup(frame: &mut Frame, area: Rect, popup: &Popup, title: &str, checked: Option<&[bool]>) {
    let matches = popup.matches();
    let width = area.width.saturating_sub(8).clamp(20, 72).min(area.width);
    let wanted = matches.len() as u16 + 4;
    let height = wanted
        .clamp(6, area.height.saturating_sub(2).max(6))
        .min(area.height);
    let place = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let title = format!("{title} · {}/{}", matches.len(), popup.options.len());
    let frame_block = block(&title, true);
    let inner = frame_block.inner(place);
    let rows = inner.height.saturating_sub(1) as usize;
    let first = popup
        .selected
        .saturating_sub(rows / 2)
        .min(matches.len().saturating_sub(rows));
    let mut lines = vec![Line::from(vec![
        Span::styled(tt!("Search: ", "Поиск: "), dim()),
        Span::raw(popup.filter.text().to_string()),
    ])];

    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            tt!("  nothing found", "  ничего не найдено"),
            dim(),
        )));
    }

    for (position, option) in matches.iter().enumerate().skip(first).take(rows) {
        let selected = position == popup.selected;
        let style = if selected {
            Style::new()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        let marker = if selected { "▸ " } else { "  " };

        let check = match checked.map(|flags| flags[*option]) {
            Some(true) => "[x] ",
            Some(false) => "[ ] ",
            None => "",
        };

        lines.push(Line::from(Span::styled(
            format!("{marker}{check}{}", popup.options[*option]),
            style,
        )));
    }

    frame.render_widget(Clear, place);
    frame.render_widget(frame_block, place);
    frame.render_widget(Paragraph::new(lines), inner);
    frame.set_cursor_position((inner.x + 7 + popup.filter.cursor() as u16, inner.y));
}

fn draw_sidebar(frame: &mut Frame, area: Rect, app: &App) {
    let lines: Vec<Line> = Page::ALL
        .iter()
        .enumerate()
        .map(|(index, page)| {
            let current = index == app.page;
            let (mark, style) = if current {
                ("▸", accent())
            } else if app.visited[index] && !app.draft.page_errors(*page).is_empty() {
                ("✗", error_style())
            } else if app.visited[index] {
                ("✓", Style::new().fg(Color::Green))
            } else {
                ("·", dim())
            };
            let text_style = match (current, app.pane) {
                (true, Pane::Menu) => Style::new()
                    .fg(Color::Black)
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD),
                (true, Pane::Fields) => accent(),
                _ => Style::new(),
            };

            Line::from(vec![
                Span::styled(format!(" {mark} "), style),
                Span::styled(format!("{}. {}", index + 1, page.title()), text_style),
            ])
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).block(block(tt!("Steps", "Шаги"), app.pane == Pane::Menu)),
        area,
    );
}

struct View {
    lines: Vec<Line<'static>>,
    focus: (usize, usize),
    cursor: Option<(usize, usize)>,
    scroll: Option<usize>,
}

fn page_view(app: &App, width: usize) -> View {
    let page = app.current_page();
    let mut lines = Vec::new();

    if page == Page::Packages {
        return packages_view(app);
    }

    if page == Page::Summary {
        lines.extend(summary_lines(app));
    }

    let show_errors = app.visited[app.page];
    let focused = (app.pane == Pane::Fields).then(|| app.focused());
    let editing = app.editing.is_some();
    let mut focus = (0, 0);
    let mut cursor = None;

    for id in app.ids() {
        let field = app.draft.field(id);
        let start = lines.len();
        let is_focused = Some(id) == focused;
        let bar = if id == Id::Packages {
            width.saturating_sub(4).max(10)
        } else {
            width.saturating_sub(4).clamp(10, BAR_WIDTH)
        };
        let (mut rows, column) = field_lines(field, is_focused, editing, bar);

        if id == Id::Layout {
            rows.splice(1..1, layout_lines(&app.draft));
        }

        if is_focused {
            cursor = column.map(|(column, row)| (column, start + row));
        }

        lines.extend(rows);
        lines.push(note(&app.draft, field, show_errors));
        lines.push(Line::raw(""));

        if is_focused {
            focus = (start, lines.len() - 1);
        }
    }

    View {
        lines,
        focus,
        cursor,
        scroll: None,
    }
}

/// Устройства разметки одной таблицей, как их увидит установщик.
fn layout_lines(draft: &Draft) -> Vec<Line<'static>> {
    let described = draft.layout().map(|layout| layout.describe());

    match described {
        Ok(lines) if !lines.is_empty() => lines
            .into_iter()
            .map(|line| Line::from(vec![Span::styled("  │ ", accent()), Span::raw(line)]))
            .collect(),
        _ => vec![Line::from(Span::styled(
            tt!("  │ the layout is empty", "  │ разметка пуста"),
            dim(),
        ))],
    }
}

fn note(draft: &Draft, field: &Field, show_errors: bool) -> Line<'static> {
    match draft.error(field.id) {
        Some(error) if show_errors => {
            Line::from(Span::styled(format!("  ✗ {error}"), error_style()))
        }
        _ => Line::from(Span::styled(format!("  {}", field.hint), dim())),
    }
}

/// Строки поля и, для полей ввода, позиция курсора (столбец, строка внутри поля).
fn field_lines(
    field: &Field,
    focused: bool,
    editing: bool,
    bar: usize,
) -> (Vec<Line<'static>>, Option<(usize, usize)>) {
    let marker = match (focused, editing) {
        (true, true) => "✎ ",
        (true, false) => "▸ ",
        _ => "  ",
    };
    let label_style = if focused { accent() } else { Style::new() };
    let mut lines = vec![Line::from(vec![
        Span::styled(marker, accent()),
        Span::styled(field.label, label_style),
    ])];
    let mut cursor = None;

    match field.kind {
        Kind::Text | Kind::Secret => {
            lines.push(input_bar(field, bar, focused, editing));
            cursor = (focused && editing).then(|| (2 + visible_cursor(field, bar), 1));
        }
        Kind::List => {
            lines.extend(field.items.iter().map(|item| {
                Line::from(vec![
                    Span::styled("  • ", accent()),
                    Span::raw(item.clone()),
                ])
            }));
            cursor = (focused && editing).then(|| (2 + visible_cursor(field, bar), lines.len()));
            lines.push(input_bar(field, bar, focused, editing));
        }
        Kind::Layout => {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!(
                        "{:<width$}",
                        tt!("Open the editor ▸", "Открыть редактор ▸"),
                        width = bar
                    ),
                    bar_style(focused),
                ),
            ]));
        }
        Kind::Select | Kind::Multi => lines.push(select_bar(field, bar, focused)),
        Kind::Viewer => {}
    }

    (lines, cursor)
}

fn shown_text(field: &Field) -> String {
    match field.kind {
        Kind::Secret => field.value().chars().map(|_| SECRET_DOT).collect(),
        _ => field.value().to_string(),
    }
}

fn visible_cursor(field: &Field, bar: usize) -> usize {
    field.input.cursor().min(bar - 1)
}

fn input_bar(field: &Field, bar: usize, focused: bool, editing: bool) -> Line<'static> {
    let text = shown_text(field);
    let start = (field.input.cursor() + 1).saturating_sub(bar);
    let shown: String = text.chars().skip(start).take(bar).collect();
    let padding = bar.saturating_sub(shown.chars().count());
    let style = if editing && focused {
        Style::new().bg(BAR_EDIT).fg(Color::White)
    } else {
        bar_style(focused)
    };

    Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{shown}{}", " ".repeat(padding)), style),
    ])
}

fn select_bar(field: &Field, bar: usize, focused: bool) -> Line<'static> {
    let width = bar.saturating_sub(2);
    let shown: String = field.display().chars().take(width).collect();
    let padding = width.saturating_sub(shown.chars().count());

    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{shown}{} ▾", " ".repeat(padding)),
            bar_style(focused),
        ),
    ])
}

fn bar_style(focused: bool) -> Style {
    if focused {
        Style::new().bg(BAR).fg(Color::White)
    } else {
        Style::new().bg(Color::Rgb(34, 36, 44)).fg(Color::Gray)
    }
}

fn summary_lines(app: &App) -> Vec<Line<'static>> {
    let draft = &app.draft;
    let encryption = match draft.layout() {
        Ok(layout) if layout.has_crypt() => tr!(
            "LUKS2: devices {}, opened by the initramfs: {}",
            "LUKS2: устройств {}, открывает initramfs: {}",
            layout.crypts().len(),
            layout.early().len()
        ),
        _ => tt!("none", "нет").to_string(),
    };
    let rows = [
        (
            tt!("Disk scheme", "Схема дисков"),
            draft.field(Id::Scheme).choice_text().to_string(),
        ),
        (tt!("Hardware", "Железо"), draft.hardware.summary()),
        (
            tt!("Firmware", "Прошивка"),
            draft.field(Id::Firmware).choice_text().to_string(),
        ),
        (tt!("Repositories", "Репозитории"), source_text(draft)),
        (
            tt!("Machine", "Машина"),
            format!(
                "{} · {}",
                draft.field(Id::Hostname).value(),
                draft.field(Id::Timezone).choice_text()
            ),
        ),
        (
            tt!("User", "Пользователь"),
            draft.field(Id::User).value().to_string(),
        ),
        (tt!("Encryption", "Шифрование"), encryption),
        (
            tt!("Initrd network", "Сеть initramfs"),
            network_text(
                draft,
                Id::InitrdMode,
                Id::InitrdInterface,
                Id::InitrdAddress,
            ),
        ),
        (
            tt!("System network", "Сеть системы"),
            network_text(
                draft,
                Id::SystemMode,
                Id::SystemInterface,
                Id::SystemAddress,
            ),
        ),
    ];
    let mut rows = rows.to_vec();

    rows.insert(3, (tt!("Packages", "Пакеты"), packages_text(app)));

    if let Some(text) = fetched_text(draft) {
        rows.insert(4, (tt!("Installer", "Установщику"), text));
    }

    let mut lines: Vec<Line> = rows
        .into_iter()
        .map(|(name, value)| {
            Line::from(vec![
                Span::styled(format!("  {name:<16}"), dim()),
                Span::raw(value),
            ])
        })
        .collect();

    if let Ok(layout) = draft.layout() {
        lines.extend(
            layout
                .describe()
                .into_iter()
                .map(|line| Line::from(Span::styled(format!("    {line}"), dim()))),
        );
    }

    lines.push(Line::raw(""));

    match draft.first_error() {
        Some((page, _, error)) => lines.push(Line::from(Span::styled(
            tr!(
                "  ✗ Fix \"{}\": {error}",
                "  ✗ Исправьте «{}»: {error}",
                page.title()
            ),
            error_style(),
        ))),
        None => lines.push(Line::from(Span::styled(
            tr!(
                "  ⚠ All data on {} will be erased",
                "  ⚠ Все данные на {} будут стёрты",
                erased_disks(draft)
            ),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ))),
    }

    lines.push(Line::raw(""));
    lines
}

/// Пакеты, которые придётся доставить на сам установщик для выбранной разметки.
fn fetched_text(draft: &crate::draft::Draft) -> Option<String> {
    let layout = draft.layout().ok()?;
    let missing = crate::storage::missing(&layout, crate::exec::installed);

    (!missing.is_empty()).then(|| {
        tr!(
            "will be installed: {}",
            "будут установлены {}",
            missing.join(" ")
        )
    })
}

fn erased_disks(draft: &crate::draft::Draft) -> String {
    draft.layout().map_or_else(
        |_| tt!("the disks", "дисках").to_string(),
        |layout| layout.disks().join(", "),
    )
}

fn packages_text(app: &App) -> String {
    match app.package_count() {
        Some(count) => tr!("{count} with dependencies", "{count} с зависимостями"),
        None => tt!(
            "not calculated — open the \"Packages\" page",
            "не рассчитано — откройте страницу «Пакеты»"
        )
        .into(),
    }
}

fn packages_view(app: &App) -> View {
    let mut lines = Vec::new();
    let note = |text: String, style: Style| Line::from(Span::styled(format!("  {text}"), style));

    match &app.resolution {
        Resolution::Blocked(error) => {
            lines.push(note(
                tr!("✗ Fix this first: {error}", "✗ Сначала исправьте: {error}"),
                error_style(),
            ));
            lines.push(note(
                tt!(
                    "Come back to this page when everything is filled in.",
                    "Вернитесь на эту страницу, когда всё заполнено."
                )
                .into(),
                dim(),
            ));
        }
        Resolution::Running(_) | Resolution::Idle => lines.push(note(
            tt!(
                "Calculating dependencies with dnf… Repository metadata is being downloaded.",
                "Считаю зависимости через dnf… Загружаются метаданные репозиториев."
            )
            .into(),
            accent(),
        )),
        Resolution::Failed { error, .. } => {
            lines.push(note(
                tt!(
                    "✗ Failed to calculate the packages:",
                    "✗ Не удалось рассчитать пакеты:"
                )
                .into(),
                error_style(),
            ));
            lines.extend(
                error
                    .lines()
                    .map(|line| note(line.to_string(), error_style())),
            );
            lines.push(note(
                tt!("Enter — retry.", "Enter — повторить.").into(),
                dim(),
            ));
        }
        Resolution::Ready { packages, .. } => {
            lines.push(Line::from(vec![
                Span::styled(
                    tr!(
                        "  To be installed: {} ",
                        "  Будет установлено: {} ",
                        packages.len()
                    ),
                    accent(),
                ),
                Span::styled(
                    tt!(
                        "(with dependencies, alphabetical) · Enter — recalculate",
                        "(с зависимостями, по алфавиту) · Enter — пересчитать"
                    ),
                    dim(),
                ),
            ]));
            lines.push(Line::raw(""));

            let width = packages
                .iter()
                .map(|package| package.name.len())
                .max()
                .unwrap_or(0)
                .min(40);

            lines.extend(packages.iter().map(|package| {
                Line::from(vec![
                    Span::raw(format!("  {:<width$}  ", package.name)),
                    Span::styled(format!("{:<22}", package.version), dim()),
                    Span::styled(format!("{:<8}", package.arch), dim()),
                    Span::styled(package.repo.clone(), dim()),
                ])
            }));
        }
    }

    View {
        lines,
        focus: (0, 0),
        cursor: None,
        scroll: Some(app.package_scroll),
    }
}

fn source_text(draft: &Draft) -> String {
    let mut sources = draft.field(Id::Repos).chosen();

    sources.extend(draft.field(Id::LocalRepos).items.iter().cloned());
    tr!(
        "{} · extra packages: {}",
        "{} · доп. пакетов: {}",
        sources.join(", "),
        draft.field(Id::Packages).items.len()
    )
}

fn network_text(draft: &Draft, mode: Id, interface: Id, address: Id) -> String {
    let name = draft.field(interface).value().trim();
    let name = if name.is_empty() {
        tt!("any interface", "любой интерфейс")
    } else {
        name
    };

    match draft.field(mode).index {
        0 => tt!("none", "нет").into(),
        1 => format!("DHCP ({name})"),
        _ => tr!("{} on {name}", "{} на {name}", draft.field(address).value()),
    }
}

fn draw_installing(frame: &mut Frame, area: Rect, app: &App) {
    let progress = &app.progress;
    let ratio = if progress.total == 0 {
        0.0
    } else {
        progress.done as f64 / progress.total as f64
    };
    let [gauge, plan] = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(area);

    frame.render_widget(
        Gauge::default()
            .block(block(tt!("Installation", "Установка"), true))
            .gauge_style(Style::new().fg(ACCENT).bg(BAR))
            .ratio(ratio.clamp(0.0, 1.0))
            .label(format!("{}/{}", progress.done, progress.total)),
        gauge,
    );
    draw_plan(frame, plan, app, false);
}

fn draw_failure(frame: &mut Frame, area: Rect, app: &App, error: &str) {
    let height = (error.lines().count() as u16 + 2).min(area.height / 2);
    let [plan, details] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(height)]).areas(area);

    draw_plan(frame, plan, app, true);
    draw_result(
        frame,
        details,
        tt!("Installation failed", "Ошибка установки"),
        Color::Red,
        error,
    );
}

/// Весь план: сделанные шаги, текущий (при ошибке он и есть сбойный) и оставшиеся; список прокручивается за текущим.
fn draw_plan(frame: &mut Frame, area: Rect, app: &App, failed: bool) {
    let progress = &app.progress;
    let rows = area.height.saturating_sub(2) as usize;
    let first = progress
        .done
        .saturating_sub(rows / 2)
        .min(progress.steps.len().saturating_sub(rows));
    let lines: Vec<Line> = progress
        .steps
        .iter()
        .enumerate()
        .skip(first)
        .take(rows)
        .map(|(index, step)| {
            let (marker, style) = match index.cmp(&progress.done) {
                std::cmp::Ordering::Less => ("✓", dim()),
                std::cmp::Ordering::Equal if failed => ("✗", error_style()),
                std::cmp::Ordering::Equal => ("▸", accent()),
                std::cmp::Ordering::Greater => ("·", Style::new().fg(Color::White)),
            };

            Line::from(Span::styled(format!(" {marker} {}", short(step)), style))
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).block(block(tt!("Plan", "План"), true)),
        area,
    );
}

/// Первая строка шага: у скриптов в несколько строк остальное скрыто.
fn short(step: &str) -> String {
    match step.split_once('\n') {
        Some((first, _)) => format!("{first} …"),
        None => step.to_string(),
    }
}

fn draw_result(frame: &mut Frame, area: Rect, title: &str, color: Color, text: &str) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(color))
        .title(Span::styled(
            format!(" {title} "),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        ));

    frame.render_widget(
        Paragraph::new(text.to_string())
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::draft::tests::filled;

    fn render(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();

        terminal.draw(|frame| draw(frame, app)).unwrap();

        let buffer = terminal.backend().buffer();

        (0..height)
            .map(|row| {
                (0..width)
                    .map(|column| buffer[(column, row)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_first_page_shows_steps_and_the_disks() {
        let screen = render(&App::new(filled()), 100, 30);

        for expected in ["шаг 1 из 8", "1. Шифрование", "2. Диск", "8. Итог"]
        {
            assert!(screen.contains(expected), "нет «{expected}»:\n{screen}");
        }
    }

    #[test]
    fn the_disk_page_shows_both_disk_choices_and_the_firmware() {
        let mut app = App::new(filled());

        app.page = 1;

        let screen = render(&app, 100, 50);

        for expected in ["/dev/sda", "SSD", "Тот же диск", "UEFI"] {
            assert!(screen.contains(expected), "нет «{expected}»:\n{screen}");
        }
    }

    #[test]
    fn secrets_are_never_drawn() {
        let mut app = App::new(filled());

        app.page = 3;
        app.focus = 2;

        let screen = render(&app, 100, 40);

        assert!(screen.contains("••"), "{screen}");
        assert!(!screen.contains("pw"), "{screen}");
    }

    #[test]
    fn errors_appear_only_on_visited_pages() {
        let mut app = App::new(filled());

        app.draft.field_mut(Id::Hostname).input = crate::input::Input::with("-bad");
        app.page = 3;

        assert!(!render(&app, 100, 40).contains('✗'));

        app.visited[3] = true;

        assert!(render(&app, 100, 40).contains("✗"));
    }

    #[test]
    fn the_summary_lists_the_choices_and_warns_about_the_disk() {
        let mut app = App::new(filled());

        app.page = Page::ALL.len() - 1;

        let screen = render(&app, 100, 30);

        assert!(
            screen.contains("будут стёрты")
                && screen.contains("moonshine")
                && screen.contains("LUKS2"),
            "{screen}"
        );
    }

    #[test]
    fn the_disk_page_lists_the_layout_rows_and_the_scheme() {
        let mut draft = filled();

        draft.field_mut(Id::Scheme).index = 2;
        draft.field_mut(Id::DataDisks).checked = vec![true, true];
        draft.sync();

        let mut app = crate::testing::app_on_disk_page(draft);

        app.focus = 0;

        let screen = render(&app, 120, 50);

        for expected in [
            "Схема дисков",
            "RAID + LVM",
            "Диски для системы",
            "Уровень RAID",
            "/dev/md/md0",
            "RAID1",
            "/dev/moonshine/root",
        ] {
            assert!(screen.contains(expected), "{expected}\n{screen}");
        }
    }

    #[test]
    fn a_broken_custom_layout_shows_its_error_on_the_disk_page() {
        let mut draft = filled();

        draft.field_mut(Id::Scheme).index = 3;
        draft.field_mut(Id::Layout).items =
            vec!["partition disk=/dev/sda,size=rest,point=/".into()];

        let mut app = crate::testing::app_on_disk_page(draft);

        app.visited[app.page] = true;

        let screen = render(&app, 120, 40);

        assert!(screen.contains("✗") && screen.contains("boot"), "{screen}");
    }

    #[test]
    fn the_summary_describes_the_layout() {
        let mut draft = filled();

        draft.field_mut(Id::Scheme).index = 1;
        draft.sync();

        let mut app = App::new(draft);

        app.page = Page::ALL.len() - 1;

        let screen = render(&app, 120, 40);

        assert!(
            screen.contains("/dev/moonshine/root") && screen.contains("lvm:moonshine"),
            "{screen}"
        );
    }

    #[test]
    fn a_small_terminal_scrolls_to_the_focused_field() {
        let mut app = App::new(filled());

        app.page = 3;
        app.focus = 4;

        assert!(render(&app, 80, 14).contains("Часовой пояс"));
    }

    #[test]
    fn an_open_list_is_drawn_over_the_page_with_its_search() {
        let mut app = App::new(filled());

        app.popup = Some(Popup {
            id: Id::Disk,
            options: vec!["/dev/sda  SSD".into(), "/dev/sdb  HDD".into()],
            filter: crate::input::Input::with("hd"),
            selected: 0,
        });

        let screen = render(&app, 100, 24);

        assert!(
            screen.contains("Поиск: hd")
                && screen.contains("/dev/sdb  HDD")
                && screen.contains("1/2"),
            "{screen}"
        );
        assert!(!screen.contains("/dev/sda  SSD"));
    }

    #[test]
    fn checkable_lists_show_their_marks_and_the_summary_names_the_repositories() {
        use crate::repos::Repo;

        let repo = |id: &str, enabled| Repo {
            id: id.into(),
            name: format!("{id} name"),
            enabled,
            file: "f.repo".into(),
        };
        let mut app = App::new(crate::draft::Draft::new(crate::sources::Sources {
            repos: vec![repo("baseos", true), repo("debug", false)],
            ..crate::sources::Sources::default()
        }));

        app.page = 2;
        app.focus = 1;

        let page = render(&app, 110, 30);

        assert!(page.contains("1 из 2: baseos"), "{page}");

        app.popup = Some(Popup {
            id: Id::Repos,
            options: vec![repo("baseos", true).label(), repo("debug", false).label()],
            filter: crate::input::Input::default(),
            selected: 0,
        });

        let screen = render(&app, 110, 30);

        assert!(
            screen.contains("[x] baseos") && screen.contains("[ ] debug"),
            "{screen}"
        );

        app.popup = None;
        app.page = Page::ALL.len() - 1;

        assert!(render(&app, 110, 30).contains("baseos"));
    }

    fn package(name: &str) -> crate::resolve::Package {
        crate::resolve::Package {
            name: name.into(),
            arch: "x86_64".into(),
            version: "1.2-3.fc44".into(),
            repo: "baseos".into(),
        }
    }

    #[test]
    fn the_packages_page_shows_every_state() {
        let mut app = App::new(filled());

        app.page = 6;
        app.resolution = Resolution::Idle;
        assert!(render(&app, 100, 20).contains("Считаю зависимости"));

        app.resolution = Resolution::Blocked("неверное имя".into());
        assert!(render(&app, 100, 20).contains("Сначала исправьте: неверное имя"));

        app.resolution = Resolution::Failed {
            key: Vec::new(),
            error: "Cannot download repomd.xml".into(),
        };
        let screen = render(&app, 100, 20);

        assert!(
            screen.contains("Не удалось рассчитать")
                && screen.contains("Cannot download repomd.xml"),
            "{screen}"
        );

        app.resolution = Resolution::Ready {
            key: Vec::new(),
            packages: vec![package("bash"), package("zsh")],
        };
        let screen = render(&app, 100, 20);

        assert!(
            screen.contains("Будет установлено: 2")
                && screen.contains("bash")
                && screen.contains("1.2-3.fc44")
                && screen.contains("baseos"),
            "{screen}"
        );

        app.package_scroll = 3;
        assert!(
            !render(&app, 100, 20).contains("Будет установлено"),
            "прокрутка уводит шапку"
        );
    }

    #[test]
    fn the_summary_tells_whether_the_packages_were_calculated() {
        let mut app = App::new(filled());

        app.page = Page::ALL.len() - 1;
        assert!(render(&app, 110, 30).contains("не рассчитано"));
    }

    #[test]
    fn a_long_list_keeps_the_selection_visible() {
        let mut app = App::new(filled());
        let options: Vec<String> = (0..200).map(|number| format!("zone{number}")).collect();

        app.popup = Some(Popup {
            id: Id::Timezone,
            options,
            filter: crate::input::Input::default(),
            selected: 150,
        });

        assert!(render(&app, 100, 20).contains("▸ zone150"));
    }

    #[test]
    fn the_hint_follows_the_mode() {
        let mut app = App::new(filled());

        assert!(render(&app, 120, 24).contains("Enter изменить"));

        app.pane = Pane::Menu;
        assert!(render(&app, 120, 24).contains("Enter/→ к полям"));

        app.pane = Pane::Fields;
        app.editing = Some(String::new());
        let screen = render(&app, 120, 24);

        assert!(
            screen.contains("Правка: Enter готово") && screen.contains('✎'),
            "{screen}"
        );
    }

    #[test]
    fn installation_and_result_screens_render() {
        let mut app = App::new(filled());

        app.screen = Screen::Installing;
        app.progress.done = 1;
        app.progress.total = 4;
        app.progress.current = "sfdisk /dev/sda".into();
        app.progress.steps = vec!["sfdisk /dev/sda".into(), "mkfs.fat /dev/sda1".into()];

        let screen = render(&app, 80, 20);

        assert!(
            screen.contains("1/4") && screen.contains("sfdisk /dev/sda"),
            "{screen}"
        );

        app.screen = Screen::Failed("cryptsetup: код 1".into());
        app.progress.done = 0;

        let screen = render(&app, 80, 20);

        assert!(screen.contains("cryptsetup: код 1"), "{screen}");
        assert!(screen.contains("✗ sfdisk /dev/sda"), "{screen}");
        assert!(screen.contains("· mkfs.fat /dev/sda1"), "{screen}");
    }

    #[test]
    fn the_editor_draws_the_tree_the_verdict_and_the_form() {
        use ratatui::crossterm::event::KeyCode;

        let mut app = crate::testing::app_on_disk_page(filled());

        app.editor = Some(Editor::new(
            app.draft.layout().unwrap(),
            vec!["/dev/sda".into()],
        ));

        let screen = render(&app, 100, 30);

        for expected in [
            "Редактор разметки",
            "▾ /dev/sda",
            "/dev/sda2",
            "✓ разметка корректна",
        ] {
            assert!(screen.contains(expected), "нет «{expected}»:\n{screen}");
        }

        let editor = app.editor.as_mut().unwrap();

        editor.key(KeyCode::Down);
        editor.key(KeyCode::Char('n'));

        let screen = render(&app, 100, 30);

        assert!(screen.contains("раздел") && screen.contains("disk=/dev/sda,size=1GiB,point="));
        assert!(screen.contains("диски: /dev/sda"), "{screen}");

        let editor = app.editor.as_mut().unwrap();

        editor.key(KeyCode::Enter);

        assert!(render(&app, 100, 30).contains("✗"));
    }

    #[test]
    fn a_broken_layout_is_red_in_the_editor() {
        use ratatui::crossterm::event::KeyCode;

        let mut app = crate::testing::app_on_disk_page(filled());
        let mut editor = Editor::new(app.draft.layout().unwrap(), Vec::new());

        editor.key(KeyCode::Down);
        editor.key(KeyCode::Down);
        editor.key(KeyCode::Char('d'));
        app.draft.set_layout(editor.layout());
        app.editor = Some(editor);

        let screen = render(&app, 100, 30);

        assert!(screen.contains("✗"), "{screen}");
        assert!(!screen.contains("✓ разметка корректна"));
    }

    #[test]
    fn the_key_bar_wraps_by_width_and_follows_the_context() {
        let keys = [
            ("n", "добавить"),
            ("NUM+ NUM−", "развернуть всё"),
            ("Esc", "готово"),
        ];

        assert_eq!(keybar(&keys, 200).len(), 1);
        assert!(keybar(&keys, 24).len() > 1);

        let mut app = crate::testing::app_on_disk_page(filled());

        app.editor = Some(Editor::new(app.draft.layout().unwrap(), Vec::new()));

        let screen = render(&app, 100, 30);

        for expected in [" n ", "раздел", "NUM+ NUM−", " Esc ", "удалить диск"] {
            assert!(screen.contains(expected), "нет «{expected}»:\n{screen}");
        }
    }

    #[test]
    fn the_layout_field_shows_a_device_table_and_not_the_raw_rows() {
        let screen = render(&crate::testing::app_on_disk_page(filled()), 120, 50);

        assert!(screen.contains("│ /dev/sda1"), "{screen}");
        assert!(screen.contains("Открыть редактор"));
        assert!(!screen.contains("partition disk="));
    }

    #[test]
    fn every_wizard_page_speaks_english() {
        crate::i18n::with(crate::i18n::Lang::En, || {
            let mut app = App::new(filled());

            for page in 0..Page::ALL.len() {
                app.page = page;
                app.visited[page] = true;

                let screen = render(&app, 120, 50);

                assert!(
                    !crate::i18n::tests::cyrillic(&screen),
                    "page {page}:\n{screen}"
                );
            }

            app.page = 0;

            let screen = render(&app, 100, 30);

            for expected in ["step 1 of 8", "1. Encryption", "2. Disk", "8. Summary"] {
                assert!(screen.contains(expected), "no \"{expected}\":\n{screen}");
            }
        });
    }
}
