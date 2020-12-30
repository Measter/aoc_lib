use nom::{
    bytes::complete::{tag, take_while1},
    combinator::{map, opt},
    sequence::tuple,
    IResult,
};
use thiserror::Error;

use std::{num::ParseIntError, ops::Neg, str::FromStr};

#[derive(Debug, Error)]
#[error("Delimiter `{}` not found", .delimeter)]
pub struct DelimiterError<'delim> {
    pub delimeter: &'delim str,
}

pub fn signed_number<F>(input: &str) -> IResult<&str, Result<F, ParseIntError>>
where
    F: FromStr<Err = ParseIntError> + Neg<Output = F>,
{
    map(
        tuple((
            map(opt(tag("-")), |o: Option<&str>| o.is_some()),
            take_while1(|c: char| c.is_ascii_digit()),
        )),
        |(is_neg, num)| num.parse::<F>().map(|n| if is_neg { -n } else { n }),
    )(input)
}

pub fn unsigned_number<F>(input: &str) -> IResult<&str, Result<F, ParseIntError>>
where
    F: FromStr<Err = ParseIntError>,
{
    map(take_while1(|c: char| c.is_ascii_digit()), |num: &str| {
        num.parse::<F>()
    })(input)
}

pub fn split_pair<'input, 'delim>(
    input: &'input str,
    delim: &'delim str,
) -> Result<(&'input str, &'input str), DelimiterError<'delim>> {
    let mut parts = input.splitn(2, delim);

    if let (Some(left), Some(right)) = (parts.next(), parts.next()) {
        Ok((left, right))
    } else {
        Err(DelimiterError { delimeter: delim })
    }
}
