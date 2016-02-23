use ast::{Script, Resource, Trigger, Conjunction, Condition, Statement, UncheckedCtx, UncheckedEnv};
use values::Range;
use util::map;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::time::Duration;

extern crate serde_json;
pub type Json = self::serde_json::Value;

extern crate fxbox_taxonomy;
use self::fxbox_taxonomy::values::{ExtNumeric, Value, Temperature};
use self::fxbox_taxonomy::devices::ServiceId;

#[derive(Debug)]
pub enum StatementError {
    NotAnObject,
    InvalidDestination,
    InvalidAction,
    InvalidArgs,
}

#[derive(Debug)]
pub enum ExpressionError {
    InvalidStructure,
    InvalidNumber,
    InvalidVendor,
    InvalidAdapter,
    InvalidKind,
}

#[derive(Debug)]
pub enum ConditionError {
    NotAnObject,
    InvalidInput,
    InvalidCapability,
    InvalidNotIn,
    InvalidRange,
}

#[derive(Debug)]
pub enum ConjunctionError {
    NotAnArray,
}

#[derive(Debug)]
pub enum TriggerError {
    NotAnObject,
    NoCondition,
    NoAction,
}

#[derive(Debug)]
pub enum RequirementError {
    NotAnObject,
    NoKind,
    InvalidInput,
    InvalidOutput,
}


#[derive(Debug)]
pub enum ResourceError {
    NotAnArray,
    InvalidResource,
}

#[derive(Debug)]
pub enum ScriptError {
    NotAnObject,
    NoRequirements,
    NoAllocations,
    NoRules,
}

#[derive(Debug)]
pub enum ValueError {
    InvalidNumber,
    InvalidVendor,
    InvalidAdapter,
    InvalidKind,
    InvalidField(String),
    NoValue,
    InvalidType,
    InvalidStructure,
}

#[derive(Debug)]
pub enum Error {
    Expression(ExpressionError),
    Statement(StatementError),
    Condition(ConditionError),
    Conjunction(ConjunctionError),
    Trigger(TriggerError),
    Requirement(RequirementError),
    Resource(ResourceError),
    Script(ScriptError),
    Value(ValueError),
}

// FIXME: Reading from a json::Parser instead of a json::Json would let us attach a position in the source code.

pub struct Parser;
impl Parser {
    /// Parse a Json object into an unchecked script.
    pub fn parse(source: Json) -> Result<Script<UncheckedCtx, UncheckedEnv>, Error> {
        Self::parse_script(source)
    }

    pub fn parse_script(source: Json) -> Result<Script<UncheckedCtx, UncheckedEnv>, Error> {
        use self::serde_json::Value::*;
        if let Object(mut obj) = source {
            let rules = if let Some(Array(rules)) = obj.remove(&"rules".to_owned()) {
                try!(map(rules, |rule| {
                    Self::parse_trigger(rule)
                }))
            } else {
                return Err(Error::Script(ScriptError::NoRules));
            };

            Ok(Script {
                metadata: (),
                rules: rules,
                phantom: PhantomData,
            })
        } else {
            Err(Error::Script(ScriptError::NotAnObject))
        }
    }

    pub fn parse_input_request(source: Json) -> Result<InputRequest, Error> {
        use self::serde_json::Value::*;
        if let Object(mut obj) = source {
            let id = match obj.remove(&"id".to_owned()) {
                None => Exactly::Empty,
                Some(String(s)) => Exactly::Exactly(s),
                _ => return Err(Error::Request(RequestError::BadId))
            };
            let parent = match obj.remove(&"parent".to_owned()) {
                None => Exactly::Empty,
                Some(String(s)) => Exactly::Exactly(s),
                _ => return Err(Error::Request(RequestError::BadParent))
            };
            let parent = match obj.remove(&"parent".to_owned()) {
                None => Exactly::Empty,
                Some(String(s)) => Exactly::Exactly(s),
                _ => return Err(Error::Request(RequestError::BadParent))
            };
            
            Ok(InputRequest {
                id: id,
                parent: parent,
                tags: tags,
                kind: kind,
                poll: poll,
                trigger: trigger
            })
        } else {
            Err(Error::Script(RequestError::NotAnObject))
        }        
    }

    /// A resource is represented by an array of id.
    pub fn parse_resource<IO>(source: Json) -> Result<Resource<IO, UncheckedCtx, UncheckedEnv>, Error> {
        use self::serde_json::Value::*;
        if let Array(services) = source {
            let services = try!(map(services, |service| {
                match service {
                    String(id) => Ok(ServiceId::new(id)),
                    _ => Err(Error::Resource(ResourceError::InvalidResource))
                }
            }));
            Ok(Resource {
                kind: (),
                services: services,
                phantom: PhantomData,
            })
        } else {
            Err(Error::Resource(ResourceError::NotAnArray))
        }
    }

    pub fn parse_trigger(source: Json) -> Result<Trigger<UncheckedCtx, UncheckedEnv>, Error> {
        use self::serde_json::Value::*;
        if let Object(mut obj) = source {
            let condition = if let Some(condition) = obj.remove(&"when".to_owned()) {
                try!(Self::parse_conjunction(condition))
            } else {
                return Err(Error::Trigger(TriggerError::NoCondition))
            };

            let execute = if let Some(Array(execute)) = obj.remove(&"do".to_owned()) {
                try!(map(execute, |statement| {
                    Self::parse_statement(statement)
                }))
            } else {
                return Err(Error::Trigger(TriggerError::NoAction))
            };

            Ok(Trigger {
                condition: condition,
                execute: execute,
                phantom: PhantomData,
            })
        } else {
            Err(Error::Trigger(TriggerError::NotAnObject))
        }
    }


    pub fn parse_conjunction(source: Json) -> Result<Conjunction<UncheckedCtx, UncheckedEnv>, Error> {
        use self::serde_json::Value::*;
        if let Array(all) = source {
            let all = try!(map(all, |condition| {
                Self::parse_condition(condition)
            }));
            Ok(Conjunction {
                all: all,
                state: (),
                phantom: PhantomData,
            })
        } else {
            Err(Error::Conjunction(ConjunctionError::NotAnArray))
        }
    }

    pub fn parse_condition(source: Json) -> Result<Condition<UncheckedCtx, UncheckedEnv>, Error> {
        use self::serde_json::Value::*;
        if let Object(mut obj) = source {
            let input = match obj.remove("service") {
                Some(service) => try!(parse_service(service)),
                _ => return Err(Error::Condition(ConditionError::InvalidInput))
            };
            let range = match obj.remove("range") {
                None => Range::Any,
                Some(Array(mut a)) =>
                // Unfortunately, no pattern-matching on arrays yet.
                    match a.len() {
                        2 => {
                            let max = a.pop().unwrap();
                            let min = a.pop().unwrap();
                            if min == Null {
                                Range::Leq(try!(Self::parse_value(max)))
                            } else if max == Null {
                                Range::Geq(try!(Self::parse_value(min)))
                            } else {
                                Range::BetweenEq {
                                    min: try!(Self::parse_value(min)),
                                    max: try!(Self::parse_value(max))
                                }
                            }
                        }
                        3 => {
                            let max = a.pop().unwrap();
                            let min = a.pop().unwrap();
                            let tag = a.pop().unwrap();
                            if let String(s) = tag {
                                if &*s == "notin" {
                                    Range::OutOfStrict {
                                        min: try!(Self::parse_value(min)),
                                        max: try!(Self::parse_value(max)),
                                    }
                                } else {
                                    return Err(Error::Condition(ConditionError::InvalidNotIn))
                                }
                            } else {
                                return Err(Error::Condition(ConditionError::InvalidNotIn))
                            }
                        }
                        _ => return Err(Error::Condition(ConditionError::InvalidRange))
                    },
                Some(val) => Range::Eq(try!(Self::parse_value(val))),
            };
            Ok(Condition {
                input: input,
                range: range,
                state: (),
                phantom: PhantomData,
            })
        } else {
            Err(Error::Condition(ConditionError::NotAnObject))
        }
    }


    pub fn parse_statement(source: Json) -> Result<Statement<UncheckedCtx, UncheckedEnv>, Error> {
        use self::serde_json::Value::*;
        if let Object(mut obj) = source {
            let destination = match obj.remove("output") {
                Some(U64(destination)) => destination as usize,
                _ => return Err(Error::Statement(StatementError::InvalidDestination))
            };
            let action = match obj.remove("capability") {
                Some(String(action)) => action,
                _ => return Err(Error::Statement(StatementError::InvalidAction))
            };
            let args = match obj.remove("args") {
                None => HashMap::new(),
                Some(Object(obj)) => {
                    let mut args = HashMap::new();
                    for (key, expr) in obj {
                        args.insert(key, try!(Self::parse_expression(expr)));
                    }
                    args
                }
                _ => {
                    return Err(Error::Statement(StatementError::InvalidArgs))
                }
            };
            Ok(Statement {
                destination: destination,
                action: action,
                arguments: args,
            })
        } else {
            Err(Error::Statement(StatementError::NotAnObject))
        }
    }

/*
    pub fn parse_expression(source: Json) -> Result<Expression<UncheckedCtx, UncheckedEnv>, Error> {
        use self::serde_json::Value::*;
        // FIXME: This should be entirely rewritten to take into account all values.
        // FIXME: Or perhaps use serde-json.
        let result = match source {
            Array(a) => {
                Expression::Vec(try!(map(a, |expr| {
                    Self::parse_expression(expr)
                })))
            },
            source@_ => Expression::Value(try!(Self::parse_value(source)))
        };
        Ok(result)
    }
     */
    
    pub fn parse_value(source: Json) -> Result<Value, Error> { // FIXME: Handle other value kinds
        use self::serde_json::Value::*;
        let result = match source {
            String(s) => Value::String(s),
            Bool(b) => Value::Bool(b),
            Object(mut obj) => {
                if obj.len() == 0 {
                    Value::Unit
                } else {
                    match obj.remove("type") {
                        Some(String(typ)) => {
                            match &*typ {
                                "ExtNumeric" => {
                                    let value = match obj.remove("value") {
                                        Some(U64(num)) => num as f64,
                                        Some(I64(num)) => num as f64,
                                        Some(F64(num)) => num,
                                        _ => return Err(Error::Value(ValueError::InvalidNumber))
                                    };
                                    let vendor = match obj.remove("vendor") {
                                        Some(String(s)) => s,
                                        None => "<unknown vendor>".to_owned(),
                                        _ => return Err(Error::Value(ValueError::InvalidVendor))
                                    };
                                    let adapter = match obj.remove("adapter") {
                                        Some(String(s)) => s,
                                        None => "<unknown adapter>".to_owned(),
                                        _ => return Err(Error::Value(ValueError::InvalidAdapter))
                                    };
                                    let kind = match obj.remove("kind") {
                                        Some(String(s)) => s,
                                        _ => return Err(Error::Value(ValueError::InvalidKind))
                                    };
                                    Value::ExtNumeric(ExtNumeric {
                                        value: value,
                                        vendor: vendor,
                                        adapter: adapter,
                                        kind: kind,
                                    })
                                },
                                "Duration" => {
                                    let sec = match obj.remove("s") {
                                        Some(U64(sec)) => sec,
                                        None => 0,
                                        _ => return Err(Error::Value(ValueError::InvalidField("s".to_owned())))
                                    };
                                    let ns = match obj.remove("nss") {
                                        Some(U64(ns)) => ns,
                                        None => 0,
                                        _ => return Err(Error::Value(ValueError::InvalidField("ns".to_owned())))
                                    };
                                    Value::Duration(Duration::new(sec, ns as u32))
                                },
                                "Temperature" => {
                                    let value = match obj.remove("value") {
                                        Some(U64(num)) => num as f64,
                                        Some(I64(num)) => num as f64,
                                        Some(F64(num)) => num,
                                        _ => return Err(Error::Value(ValueError::InvalidNumber))
                                    };
                                    let temp = match obj.remove("unit") {
                                        Some(String(unit)) => {
                                            match &*unit {
                                                "F" => Temperature::F(value),
                                                "C" => Temperature::C(value),
                                                _ => return Err(Error::Value(ValueError::InvalidField("unit".to_owned())))
                                            }
                                        },
                                        _ => return Err(Error::Value(ValueError::InvalidField("unit".to_owned())))
                                    };
                                    Value::Temperature(temp)
                                },
                                "Json" => {
                                    match obj.remove("value") {
                                        Some(value) => Value::Json(fxbox_taxonomy::values::Json(value)),
                                        None => return Err(Error::Value(ValueError::NoValue))
                                    }
                                },
                                "TimeStamp" => {
                                    unimplemented!()                            
                                },
                                "Color" => {
                                    unimplemented!()
                                },
                                "Binary" => {
                                    unimplemented!()
                                },
                                _ => return Err(Error::Value(ValueError::InvalidType))
                            }
                        },
                        _ => return Err(Error::Value(ValueError::InvalidType))                         }
                }
            },
            _ => return Err(Error::Value(ValueError::InvalidStructure)),
        };
        Ok(result)
    }

}
