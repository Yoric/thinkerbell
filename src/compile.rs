//! A script compiler
//!
//! This compiler take untrusted code (`Script<UncheckedCtx,
//! UncheckedEnv>`) and performs the following transformations and
//! checks:
//! - Ensure that the `Script` has at least one `Rule`.
//! - Ensure that each `Rule `has at least one `Conjunction`.
//! - Ensure that each `Conjunction` has at least one `Condition`.
//! - Transform each `Condition` to make sure that the type of the
//!   `range` matches the type of the `input`.
//! - Ensure that in each `Statement`, the type of the `value` matches
//!   the type of the `destination`.
//! - Introduce markers to keep track of which conditions were already
//!   met last time they were evaluated.

use std::marker::PhantomData;

use ast::{Script, Rule, Statement, Match, Context, UncheckedCtx};
use util::*;

use fxbox_taxonomy::api::API;

use serde::ser::{Serialize, Serializer};
use serde::de::{Deserialize, Deserializer};


/// The environment in which the code is meant to be executed.  This
/// can typically be instantiated either with actual bindings to
/// devices, or with a unit-testing framework. // FIXME: Move this to run.rs
pub trait ExecutableDevEnv: Serialize + Deserialize + Default + Send + Sync {
    type WatchGuard;
    type API: API<WatchGuard = Self::WatchGuard>;
}


///
/// # Precompilation
///

#[derive(Serialize, Deserialize)]
pub struct CompiledCtx<Env> where Env: Serialize + Deserialize {
    phantom: Phantom<Env>,
}

/// We implement `Default` to keep derive happy, but this code should
/// be unreachable.
impl<Env> Default for CompiledCtx<Env> where Env: Serialize + Deserialize {
    fn default() -> Self {
        panic!("Called CompledCtx<_>::default()");
    }
}

impl<Env> Context for CompiledCtx<Env> where Env: Serialize + Deserialize {
}

#[derive(Debug)]
pub enum SourceError {
    /// The source doesn't define any rule.
    NoRules,

    /// A rule doesn't have any statements.
    NoStatements,

    /// A rule doesn't have any condition.
    NoConditions,
}

#[derive(Debug)]
pub enum TypeError {
    /// The range cannot be typed.
    InvalidRange,

    /// The range has one type but this type is incompatible with the
    /// kind of the `Condition`.
    KindAndRangeDoNotAgree,
}

#[derive(Debug)]
pub enum Error {
    SourceError(SourceError),
    TypeError(TypeError),
}

pub struct Compiler<Env> where Env: ExecutableDevEnv {
    phantom: PhantomData<Env>,
}

impl<Env> Compiler<Env> where Env: ExecutableDevEnv {
    pub fn new() -> Result<Self, Error> {
        Ok(Compiler {
            phantom: PhantomData
        })
    }

    pub fn compile(&self, script: Script<UncheckedCtx>)
                   -> Result<Script<CompiledCtx<Env>>, Error> {
        self.compile_script(script)
    }

    fn compile_script(&self, script: Script<UncheckedCtx>) -> Result<Script<CompiledCtx<Env>>, Error>
    {
        if script.rules.len() == 0 {
            return Err(Error::SourceError(SourceError::NoRules));
        }
        let rules = try!(map(script.rules, |rule| {
            self.compile_trigger(rule)
        }));
        Ok(Script {
            rules: rules,
            phantom: Phantom::new()
        })
    }

    fn compile_trigger(&self, trigger: Rule<UncheckedCtx>) -> Result<Rule<CompiledCtx<Env>>, Error>
    {
        if trigger.execute.len() == 0 {
            return Err(Error::SourceError(SourceError::NoStatements));
        }
        if trigger.conditions.len() == 0 {
            return Err(Error::SourceError(SourceError::NoConditions));
        }
        let conditions = try!(map(trigger.conditions, |match_| {
            self.compile_match(match_)
        }));
        let execute = try!(map(trigger.execute, |statement| {
            self.compile_statement(statement)
        }));
        Ok(Rule {
            conditions: conditions,
            execute: execute,
            phantom: Phantom::new()
        })
    }

    fn compile_match(&self, match_: Match<UncheckedCtx>) -> Result<Match<CompiledCtx<Env>>, Error>
    {
        let typ = match match_.range.get_type() {
            Err(_) => return Err(Error::TypeError(TypeError::InvalidRange)),
            Ok(typ) => typ
        };
        if match_.kind.get_type() != typ {
            return Err(Error::TypeError(TypeError::KindAndRangeDoNotAgree));
        }
        let source = match_.source.iter().map(|input| input.clone().with_kind(match_.kind.clone())).collect();
        Ok(Match {
            source: source,
            kind: match_.kind,
            range: match_.range,
            phantom: Phantom::new()
        })
    }

    fn compile_statement(&self, statement: Statement<UncheckedCtx>) -> Result<Statement<CompiledCtx<Env>>, Error>
    {
        let destination = statement.destination.iter().map(|output| output.clone().with_kind(statement.kind.clone())).collect();
        Ok(Statement {
            destination: destination,
            value: statement.value,
            kind: statement.kind,
            phantom: Phantom::new()
        })
    }
}
