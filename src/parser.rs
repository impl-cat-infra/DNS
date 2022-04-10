use nom::{
    bits,
    branch::alt,
    bytes::complete::{tag, take},
    combinator::{eof, flat_map, map, map_res, verify},
    error::Error,
    multi::{count, many_till},
    number::complete::{be_u16, be_u32, be_u8},
    sequence::tuple,
    IResult,
};
use nom_derive::Nom;
use num_enum::TryFromPrimitive;
use std::borrow::Cow;

#[derive(TryFromPrimitive, Debug, Clone, Copy)]
#[repr(u8)]
pub enum OpCode {
    Query = 0,
    IQuery = 1,
    Status = 2,
}

#[derive(TryFromPrimitive, Nom, Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u16)]
pub enum Type {
    A = 1,
    NS = 2,
    CNAME = 5,
    SOA = 6,
    PTR = 12,
    MX = 15,
    TXT = 16,
    AAAA = 28,

    OPT = 41,

    AXFR = 252,
    ANY = 255,
}

impl Type {
    pub fn need_recursive(&self) -> bool {
        match self {
            // This may lead to inconsistency, but let's offload those footguns to our user anyway
            Self::NS => true,
            Self::SOA => true,

            _ => false,
        }
    }
}

#[derive(Debug)]
pub struct Name<'a> {
    pub labels: Vec<Cow<'a, str>>,
    pub ptr: Option<u16>,
}

#[derive(Debug)]
pub struct Question<'a> {
    pub name: Name<'a>,
    pub ty: Type,
    // Right now, silently ignores QCLASS
}

#[derive(Debug)]
pub struct RR<'a> {
    pub name: Name<'a>,
    pub ty: Type,
    // Right now, silently ignores CLASS
    pub ttl: u32,
    pub rdata: &'a [u8],
}

#[derive(Debug)]
pub struct ReqHeaderStatus {
    pub qr: bool,
    pub opcode: OpCode,

    pub rd: bool,

    pub ad: bool,
    pub cd: bool,
}

#[derive(Debug)]
pub struct ReqHeader {
    pub id: u16,

    pub status: ReqHeaderStatus,

    pub qdcnt: u16,
    pub arcnt: u16, // We are only looking for EDNS0
}

#[derive(Debug)]
pub struct Req<'a> {
    pub header: ReqHeader,
    pub questions: Vec<Question<'a>>,
    pub additionals: Vec<RR<'a>>,
}

pub fn parse_header_status(input: &[u8]) -> IResult<&[u8], ReqHeaderStatus> {
    let parser = tuple::<_, _, Error<(&[u8], usize)>, _>((
        bits::complete::take(1usize),   // QR
        bits::complete::take(4usize),   // OPCODE
        bits::complete::tag(0, 2usize), // AA + TC
        bits::complete::take(1usize),   // RD
        bits::complete::tag(0, 2usize), // RA + Z(1)
        bits::complete::take(1usize),   // AD
        bits::complete::take(1usize),   // CD
        bits::complete::tag(0, 4usize), // RCODE
    ));

    map_res(bits::bits(parser), |(qr, opcode_raw, _, rd, _, ad, cd, _): (u8, u8, _, u8, _, u8, u8, _)| -> Result<_, <OpCode as TryFrom<u8>>::Error> {
        Ok(ReqHeaderStatus {
            qr: qr != 0,
            opcode: OpCode::try_from(opcode_raw)?,
            rd: rd != 0,
            ad: ad != 0,
            cd: cd != 0,
        })
    })(input)
}

fn parse_header(input: &[u8]) -> IResult<&[u8], ReqHeader> {
    let parser = tuple((
        be_u16,
        parse_header_status,
        be_u16,
        count(tag(b"\0\0"), 2),
        be_u16,
    ));

    map(parser, |(id, status, qdcnt, _, arcnt)| ReqHeader {
        id,
        status,
        qdcnt,
        arcnt,
    })(input)
}

fn parse_label<'a>(input: &'a [u8]) -> IResult<&'a [u8], Cow<'a, str>> {
    map(flat_map(be_u8, |cnt| take(cnt)), |slice| {
        String::from_utf8_lossy(slice)
    })(input)
}

fn parse_ptr<'a>(input: &'a [u8]) -> IResult<&'a [u8], Option<u16>> {
    alt((
        map(tag(b"\0"), |_| None),
        map(verify(be_u16, |parsed| (parsed >> 14) == 3), Option::Some),
    ))(input)
}

fn parse_name<'a>(input: &'a [u8]) -> IResult<&'a [u8], Name<'a>> {
    map(many_till(parse_label, parse_ptr), |(labels, ptr)| Name {
        labels,
        ptr,
    })(input)
}

fn parse_question<'a>(input: &'a [u8]) -> IResult<&'a [u8], Question<'a>> {
    use nom_derive::Parse;
    map(
        tuple((parse_name, Type::parse, be_u16)),
        |(name, ty, _cls)| Question { name, ty },
    )(input)
}

fn parse_rr<'a>(input: &'a [u8]) -> IResult<&'a [u8], RR<'a>> {
    use nom_derive::Parse;
    map(
        tuple((
            parse_name,
            Type::parse,
            be_u16,                 // Class
            be_u32,                 // TTL
            flat_map(be_u16, take), // RDLENGRTH + RDATA
        )),
        |(name, ty, _cls, ttl, rdata)| RR {
            name,
            ty,
            ttl,
            rdata,
        },
    )(input)
}

fn parse_request<'a>(input: &'a [u8]) -> IResult<&'a [u8], Req<'a>> {
    let (input, hdr) = parse_header(input)?;
    let (input, questions) = count(parse_question, hdr.qdcnt as usize)(input)?;
    let (input, additionals) = count(parse_rr, hdr.arcnt as usize)(input)?;

    Ok((
        input,
        Req {
            header: hdr,
            questions,
            additionals,
        },
    ))
}

pub fn parse<'a>(input: &'a [u8]) -> IResult<&'a [u8], Req<'a>> {
    map(tuple((parse_request, eof)), |(res, _)| res)(input)
}
