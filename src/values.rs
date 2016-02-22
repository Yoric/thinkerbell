extern crate fxbox_taxonomy;
use self::fxbox_taxonomy::values::{Value, Type};

#[derive(Clone)]
pub enum Range {
    /// Leq(x) accepts any value v such that v <= x.
    Leq(Value),

    /// Geq(x) accepts any value v such that v >= x.
    Geq(Value),

    /// BetweenEq {min, max} accepts any value v such that `min <= v`
    /// and `v <= max`. If `max < min`, it never accepts anything.
    BetweenEq {min:Value, max:Value},

    /// OutOfStrict {min, max} accepts any value v such that `v < min`
    /// or `max < v`
    OutOfStrict {min:Value, max:Value},


    Eq(Value),


    /// `Any` accepts all values.
    Any,
}

impl Range {
    pub fn contains(&self, value: &Value) -> bool {
        use self::Range::*;
        match *self {
            Leq(ref max) => value <= max,
            Geq(ref min) => value >= min,
            BetweenEq {ref min, ref max} => min <= value && value <= max,
            OutOfStrict {ref min, ref max} => value < min || max < value,
            Eq(ref val) => value == val,
            Any => true
        }
    }

    pub fn get_type(&self) -> Result<Option<Type>, ()> {
        use self::Range::*;
        match *self {
            Leq(ref v) | Geq(ref v) | Eq(ref v) => Ok(Some(v.get_type())),
            BetweenEq{ref min, ref max} | OutOfStrict{ref min, ref max} => {
                let min_typ = min.get_type();
                let max_typ = max.get_type();
                if min_typ == max_typ {
                    Ok(Some(min_typ))
                } else {
                    Err(())
                }
            }
            Any => Ok(None)
        }
    }
}
