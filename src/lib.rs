pub extern crate clap;
pub extern crate promkit;
use clap::{ArgMatches, Command};
use std::collections::HashMap;

use promkit::{
    buffer::Buffer,
    build::Builder,
    crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers},
    crossterm::style::Color,
    grapheme::Graphemes,
    handler,
    keybind::KeyBind,
    readline::{self, State},
    register::Register,
    suggest::Suggest,
    EventHandleFn,
};

#[derive(Debug, Eq, PartialEq)]
pub enum TaskAction {
    Continue,
    Break,
    Exit,
}

pub trait Task {
    fn command(&self) -> Command;
    fn action(&self, matches: &ArgMatches) -> TaskAction;
    fn suggests(&self, args: &[&'_ str]) -> Option<Graphemes>;
}

pub fn complete<L: IntoIterator<Item = String>>(l: L, text: &str) -> Graphemes {
    let mut s = Suggest::default();
    s.register_all(l);
    let g = Graphemes::from(text);
    s.search(&g).unwrap_or(g)
}

pub struct Cli {
    cmd: Command,
    cmds: HashMap<String, Box<dyn Task + 'static>>,
}

impl Cli {
    pub fn new(name: impl Into<clap::builder::Str>) -> Self {
        // strip out usage
        const PARSER_TEMPLATE: &str = "\
        {all-args}
    ";
        Cli {
            cmd: Command::new(name)
                .multicall(true)
                .arg_required_else_help(true)
                .subcommand_required(true)
                .subcommand_value_name("APPLET")
                .subcommand_help_heading("Commands")
                .help_template(PARSER_TEMPLATE),
            cmds: HashMap::new(),
        }
    }

    pub fn add_task<T: Task + 'static>(mut self, t: T) -> Self {
        self.cmds
            .insert(t.command().get_name().to_string(), Box::new(t));
        self
    }

    pub fn parse(&self, line: &str) -> Result<Option<ArgMatches>, String> {
        let args = shlex::split(line).ok_or("error: Invalid quoting")?;
        if args.is_empty() {
            return Ok(None);
        }
        let cmd = self.command();
        cmd.try_get_matches_from(args)
            .map(|r| Some(r))
            .map_err(|e| e.to_string())
    }

    pub fn run(&self, line: &str) -> Result<TaskAction, String> {
        let matches = match self.parse(line)? {
            None => return Ok(TaskAction::Continue),
            Some(m) => m,
        };
        Ok(self.action(&matches))
    }

    pub fn prompt(self: &std::sync::Arc<Self>) -> readline::Builder {
        let mut b = KeyBind::default();
        let cli = self.clone();
        b.assign(vec![
            (
                Event::Key(KeyEvent {
                    code: KeyCode::Tab,
                    modifiers: KeyModifiers::NONE,
                }),
                Box::new({
                    move |_, _, _: &mut std::io::Stdout, state: &mut State| {
                        let line = state.0.editor.data.to_string();
                        let line = line.split_whitespace().collect::<Vec<_>>();
                        if let Some(r) = cli.suggests(&line) {
                            state.0.editor.replace(&r)
                        }
                        Ok(false)
                    }
                }) as Box<EventHandleFn<State>>,
            ),
            (
                Event::Key(KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers: KeyModifiers::CONTROL,
                }),
                Box::new(|_, _, out: &mut std::io::Stdout, state: &mut State| {
                    state.0.editor = Box::new(Buffer::default());
                    handler::enter()(None, None, out, state)
                }) as Box<EventHandleFn<State>>,
            ),
        ]);

        readline::Builder::default().handler(b)
    }

    pub fn run_batch(&self, cmd: &str) -> Result<(), String> {
        for line in cmd
            .split('\n')
            .map(|s| s.trim())
            .flat_map(|s| s.split(';').map(|s| s.trim()))
        {
            self.run(line)?;
        }
        Ok(())
    }

    pub fn run_interactive_with<F: Fn(readline::Builder) -> readline::Builder>(
        self: &std::sync::Arc<Self>,
        f: F,
    ) -> Result<TaskAction, String> {
        let mut prompt = f(self.prompt()).build().map_err(|e| e.to_string())?;
        loop {
            let line = prompt.run().map_err(|e| e.to_string())?;
            let action = self.run(&line).unwrap_or_else(|e| {
                println!("{}", e);
                TaskAction::Continue
            });
            if action != TaskAction::Continue {
                break Ok(action);
            }
        }
    }

    pub fn run_interactive(self: &std::sync::Arc<Self>) -> Result<TaskAction, String> {
        self.run_interactive_with(|b| {
            b.label(&format!("{}> ", self.cmd.get_name()))
                .label_color(Color::Reset)
                .limit_history_size(3)
        })
    }
}

impl Task for Cli {
    fn command(&self) -> Command {
        // strip out name/version
        const APPLET_TEMPLATE: &str = "\
                {about-with-newline}\n\
                {usage-heading}\n    {usage}\n\
                \n\
                {all-args}{after-help}\
            ";
        self.cmds.values().fold(self.cmd.clone(), |c, t| {
            c.subcommand(t.command().help_template(APPLET_TEMPLATE))
        })
    }
    fn action(&self, matches: &ArgMatches) -> TaskAction {
        let (name, matches) = matches.subcommand().unwrap();
        self.cmds[name].action(&matches)
    }
    fn suggests(&self, args: &[&'_ str]) -> Option<Graphemes> {
        args.iter().next().map(|a| {
            self.cmds
                .get(*a)
                .map(|c| c.suggests(&args[1..]))
                .flatten()
                .map_or_else(
                    || {
                        complete(
                            self.cmds
                                .keys()
                                .map(move |s| s.to_string())
                                .chain(["help".to_string()]),
                            *a,
                        )
                    },
                    |r| Graphemes::from([*a, &r.to_string()].join(" ")),
                )
        })
    }
}
