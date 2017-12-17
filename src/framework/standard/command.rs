use std::sync::Arc;
use super::{Args, Configuration};
use client::Context;
use model::{Message, Permissions};
use std::collections::HashMap;
use std::fmt;

pub type Check = Fn(&mut Context, &Message, &mut Args, &Arc<Command>) -> bool
                     + Send
                     + Sync
                     + 'static;
pub type Exec = Fn(&mut Context, &Message, Args) -> Result<(), Error> + Send + Sync + 'static;
pub type Help = Fn(&mut Context, &Message, HashMap<String, Arc<CommandGroup>>, Args)
                   -> Result<(), Error>
                    + Send
                    + Sync
                    + 'static;
pub type BeforeHook = Fn(&mut Context, &Message, &str) -> bool + Send + Sync + 'static;
pub type AfterHook = Fn(&mut Context, &Message, &str, Result<(), Error>) + Send + Sync + 'static;
pub(crate) type InternalCommand = Arc<Command>;
pub type PrefixCheck = Fn(&mut Context, &Message) -> Option<String> + Send + Sync + 'static;

pub enum CommandOrAlias {
    Alias(String),
    Command(InternalCommand),
}

/// An error from a command.
#[derive(Clone, Debug)]
pub struct Error(pub String);

// TODO: Have seperate `From<(&)String>` and `From<&str>` impls via specialization
impl<D: fmt::Display> From<D> for Error {
    fn from(d: D) -> Self {
        Error(format!("{}", d))
    }
}

/// Command function type. Allows to access internal framework things inside
/// your commands.
pub enum CommandType {
    StringResponse(String),
    Basic(Box<Exec>),
    WithCommands(Box<Help>),
}

pub struct CommandGroup {
    pub prefix: Option<String>,
    pub commands: HashMap<String, CommandOrAlias>,
    /// Some fields taken from Command
    pub bucket: Option<String>,
    pub required_permissions: Permissions,
    pub allowed_roles: Vec<String>,
    pub help_available: bool,
    pub dm_only: bool,
    pub guild_only: bool,
    pub owners_only: bool,
}

/// Command struct used to store commands internally.
pub struct Command {
    /// A set of checks to be called prior to executing the command. The checks
    /// will short-circuit on the first check that returns `false`.
    pub checks: Vec<Box<Check>>,
    /// Function called when the command is called.
    pub exec: CommandType,
    /// Ratelimit bucket.
    pub bucket: Option<String>,
    /// Command description, used by other commands.
    pub desc: Option<String>,
    /// Example arguments, used by other commands.
    pub example: Option<String>,
    /// Command usage schema, used by other commands.
    pub usage: Option<String>,
    /// Minumum amount of arguments that should be passed.
    pub min_args: Option<i32>,
    /// Maximum amount of arguments that can be passed.
    pub max_args: Option<i32>,
    /// Permissions required to use this command.
    pub required_permissions: Permissions,
    /// Roles allowed to use this command.
    pub allowed_roles: Vec<String>,
    /// Whether command should be displayed in help list or not, used by other commands.
    pub help_available: bool,
    /// Whether command can be used only privately or not.
    pub dm_only: bool,
    /// Whether command can be used only in guilds or not.
    pub guild_only: bool,
    /// Whether command can only be used by owners or not.
    pub owners_only: bool,
    pub(crate) aliases: Vec<String>,
}

impl Command {
    pub fn new<F>(f: F) -> Self
        where F: Fn(&mut Context, &Message, Args) -> Result<(), Error> + Send + Sync + 'static {
        Command {
            exec: CommandType::Basic(Box::new(f)),
            ..Command::default()
        }
    }
}

impl Default for Command {
    fn default() -> Command {
        Command {
            aliases: Vec::new(),
            checks: Vec::default(),
            exec: CommandType::Basic(Box::new(|_, _, _| Ok(()))),
            desc: None,
            usage: None,
            example: None,
            min_args: None,
            bucket: None,
            max_args: None,
            required_permissions: Permissions::empty(),
            dm_only: false,
            guild_only: false,
            help_available: true,
            owners_only: false,
            allowed_roles: Vec::new(),
        }
    }
}

pub fn positions(ctx: &mut Context, msg: &Message, conf: &Configuration) -> Option<Vec<usize>> {
    if !conf.prefixes.is_empty() || conf.dynamic_prefix.is_some() {
        // Find out if they were mentioned. If not, determine if the prefix
        // was used. If not, return None.
        let mut positions: Vec<usize> = vec![];

        if let Some(mention_end) = find_mention_end(&msg.content, conf) {
            positions.push(mention_end);
            return Some(positions);
        } else if let Some(ref func) = conf.dynamic_prefix {
            if let Some(x) = func(ctx, msg) {
                if msg.content.starts_with(&x) {
                    positions.push(x.len());
                }
            } else {
                for n in &conf.prefixes {
                    if msg.content.starts_with(n) {
                        positions.push(n.len());
                    }
                }
            }
        } else {
            for n in &conf.prefixes {
                if msg.content.starts_with(n) {
                    positions.push(n.len());
                }
            }
        };

        if positions.is_empty() {
            return None;
        }

        let pos = *unsafe { positions.get_unchecked(0) };

        if conf.allow_whitespace {
            positions.insert(0, find_end_of_prefix_with_whitespace(&msg.content, pos).unwrap_or(pos));
        } else if find_end_of_prefix_with_whitespace(&msg.content, pos).is_some() {
            return None;
        }

        Some(positions)
    } else if conf.on_mention.is_some() {
        find_mention_end(&msg.content, conf).map(|mention_end| {
            vec![mention_end] // This can simply be returned without trying to find the end whitespaces as trim will remove it later
        })
    } else {
        None
    }
}

fn find_mention_end(content: &str, conf: &Configuration) -> Option<usize> {
    conf.on_mention.as_ref().and_then(|mentions| {
        mentions
            .iter()
            .find(|mention| content.starts_with(&mention[..]))
            .map(|m| m.len())
    })
}

// Finds the end of the first continuous block of whitespace after the prefix
fn find_end_of_prefix_with_whitespace(content: &str, position: usize) -> Option<usize> {
    let mut ws_split = content.split_whitespace();
    if let Some(cmd) = ws_split.nth(1) {
        if let Some(index_of_cmd) = content.find(cmd) {
            if index_of_cmd > position && index_of_cmd <= content.len() {
                let slice = unsafe { content.slice_unchecked(position, index_of_cmd) }.as_bytes();
                for byte in slice.iter() {
                    // 0x20 is ASCII for space
                    if *byte != 0x20u8 {
                        return None;
                    }
                }
                return Some(index_of_cmd);
            }
        }
    }
    None
}
