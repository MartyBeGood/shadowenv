use crate::hash::{Hash, Source};
use crate::lang;
use crate::lang::ShadowLang;
use crate::loader;
use crate::output;
use crate::shadowenv::Shadowenv;
use crate::undo;

use std::borrow::Cow;
use std::env;
use std::rc::Rc;
use std::result::Result;
use std::str::FromStr;

use failure::Error;
use shell_escape as shell;

pub enum VariableOutputMode {
    FishMode,
    PorcelainMode,
    PosixMode,
}

pub fn run(shadowenv_data: &str, mode: VariableOutputMode) -> Result<(), Error> {
    match load_env(shadowenv_data)? {
        Some((shadowenv, activation)) => {
            apply_env(&shadowenv, mode)?;
            output::print_activation_to_tty(activation, shadowenv.features());
            Ok(())
        },
        None => Ok(()),
    }
}

pub fn load_env(shadowenv_data: &str) -> Result<Option<(Shadowenv, bool)>, Error> {
    let mut parts = shadowenv_data.splitn(2, ":");
    let prev_hash = parts.next();
    let json_data = parts.next().unwrap_or("{}");

    let active: Option<Hash> = match prev_hash {
        None => None,
        Some("") => None,
        Some("0000000000000000") => None,
        Some(x) => Some(Hash::from_str(x)?),
    };

    let target: Option<Source> =
        loader::load(env::current_dir()?, loader::DEFAULT_RELATIVE_COMPONENT)?;

    match (&active, &target) {
        (None, None) => {
            return Ok(None);
        }
        (Some(a), Some(t)) if a.hash == t.hash()? => {
            return Ok(None);
        }
        (_, _) => (),
    }

    let target_hash = match &target {
        Some(t) => t.hash().unwrap_or(0),
        None => 0,
    };

    let data = undo::Data::from_str(json_data)?;
    let shadowenv = Rc::new(Shadowenv::new(env::vars().collect(), data, target_hash));

    let activation = match target {
        Some(target) => {
            if let Err(_) = ShadowLang::run_program(shadowenv.clone(), target) {
                // no need to return anything descriptive here since we already had ketos print it
                // to stderr.
                return Err(lang::ShadowlispError {}.into());
            }
            true
        }
        None => false,
    };

    let shadowenv = Rc::try_unwrap(shadowenv).unwrap();
    Ok(Some((shadowenv, activation)))
}

pub fn mutate_own_env(shadowenv: &Shadowenv) -> Result<String, Error> {
    let shadowenv_data = shadowenv.format_shadowenv_data()?;

    for (k, v) in shadowenv.exports() {
        match v {
            Some(s) => env::set_var(k, &s),
            None    => env::set_var(k, ""),
        }
    }

    Ok(shadowenv_data)
}

pub fn apply_env(shadowenv: &Shadowenv, mode: VariableOutputMode) -> Result<(), Error> {
    let shadowenv_data = shadowenv.format_shadowenv_data()?;

    match mode {
        VariableOutputMode::PosixMode => {
            println!("__shadowenv_data={}", shell_escape(&shadowenv_data));
            for (k, v) in shadowenv.exports() {
                match v {
                    Some(s) => println!("export {}={}", k, shell_escape(&s)),
                    None => println!("unset {}", k),
                }
            }
        }
        VariableOutputMode::FishMode => {
            println!("set -g __shadowenv_data {}", shell_escape(&shadowenv_data));
            for (k, v) in shadowenv.exports() {
                match v {
                    Some(s) => {
                        if k == "PATH" {
                            let pathlist = shell_escape(&s).replace(":", "' '");
                            println!("set -gx {} {}", k, pathlist);
                        } else {
                            println!("set -gx {} {}", k, shell_escape(&s));
                        }
                    }
                    None => {
                        println!("set -e {}", k);
                    }
                }
            }
        }
        VariableOutputMode::PorcelainMode => {
            // three fields: <operation> : <name> : <value>
            // opcodes: 1: set, unexported
            //          2: set, exported
            //          3: unset (value is empty)
            // field separator is 0x1F; record separator is 0x1E. There's a trailing record
            // separator because I'm lazy but don't depend on it not going away.
            print!("\x01\x1F__shadowenv_data\x1F{}\x1E", shadowenv_data);
            for (k, v) in shadowenv.exports() {
                match v {
                    Some(s) => print!("\x02\x1F{}\x1F{}\x1E", k, s),
                    None => print!("\x03\x1F{}\x1F\x1E", k),
                }
            }
        }
    }
    Ok(())
}

fn shell_escape(s: &str) -> String {
    shell::escape(Cow::from(s)).to_string()
}
