use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event;
use ratatui::crossterm::event::Event as Input;

use crate::app::Action;
use crate::app::App;
use crate::app::Event;
use crate::config::Config;
use crate::config::Ids;
use crate::draft::Draft;
use crate::error::Error;
use crate::exec;
use crate::plan;
use crate::plan::Step;
use crate::resolve;
use crate::sources::Sources;
use crate::ui;

const PRINTK: &str = "/proc/sys/kernel/printk";
const TICK: Duration = Duration::from_millis(100);

pub fn run() -> Result<(), Error> {
    let draft = Draft::new(Sources::detect());
    let console_level = silence_kernel_log();
    let mut terminal = ratatui::try_init().map_err(Error::Terminal)?;
    let result = event_loop(&mut terminal, App::new(draft));
    let restored = ratatui::try_restore().map_err(Error::Terminal);

    restore_kernel_log(console_level);

    result.and(restored)
}

fn silence_kernel_log() -> Option<String> {
    let text = fs::read_to_string(PRINTK).ok()?;
    let level = text.split_whitespace().next()?.to_string();

    set_console_level("1");

    Some(level)
}

fn restore_kernel_log(level: Option<String>) {
    if let Some(level) = level {
        set_console_level(&level);
    }
}

fn set_console_level(level: &str) {
    let _ = Command::new("dmesg")
        .args(["-n", level])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn event_loop(terminal: &mut DefaultTerminal, mut app: App) -> Result<(), Error> {
    let (sender, receiver) = mpsc::channel();

    loop {
        terminal
            .draw(|frame| ui::draw(frame, &app))
            .map_err(Error::Terminal)?;

        while let Ok(message) = receiver.try_recv() {
            app.receive(message);
        }

        if !event::poll(TICK).map_err(Error::Terminal)? {
            continue;
        }

        let Input::Key(key) = event::read().map_err(Error::Terminal)? else {
            continue;
        };

        match app.handle(key) {
            Action::None => {}
            Action::Quit => return Ok(()),
            Action::Install(config) => install(&config, sender.clone()),
            Action::Resolve(args) => resolve(args, sender.clone()),
        }
    }
}

fn resolve(args: Vec<String>, sender: Sender<Event>) {
    thread::spawn(move || {
        let result = resolve::run(&args);

        let _ = sender.send(Event::Resolved { key: args, result });
    });
}

fn install(config: &Config, sender: Sender<Event>) {
    let config = config.clone();

    thread::spawn(move || {
        let steps = plan::installation(&config, &Ids::random(&config.layout), exec::installed);
        let total = steps.len();

        let _ = sender.send(Event::Planned(steps.iter().map(Step::to_string).collect()));

        let result = exec::execute(
            &steps,
            Path::new(plan::TARGET),
            &mut exec::System,
            |index, step| {
                let _ = sender.send(Event::Progress {
                    index,
                    total,
                    step: step.to_string(),
                });
            },
        );

        let _ = sender.send(Event::Finished(result.map_err(|error| error.to_string())));
    });
}
