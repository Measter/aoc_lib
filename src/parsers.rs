use color_eyre::eyre::{eyre, Result};
use nom::{
    bytes::complete::{tag, take_while1},
    combinator::{map, opt},
    sequence::tuple,
    IResult,
};

use std::{num::ParseIntError, ops::Neg, str::FromStr};

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

pub fn split_pair<'a>(input: &'a str, delim: &str) -> Result<(&'a str, &'a str)> {
    let mut parts = input.splitn(2, delim);

    if let (Some(left), Some(right)) = (parts.next(), parts.next()) {
        Ok((left, right))
    } else {
        Err(eyre!("Delimiter `{}` not found in `{}`", delim, input))
    }
}
