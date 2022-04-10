use std::{borrow::Borrow, io::Write};

use serde::Deserialize;

#[derive(Debug, Hash, PartialEq, Eq)]
pub struct Name(Vec<String>);

impl Borrow<[String]> for Name {
    fn borrow(&self) -> &[String] {
        self.0.borrow()
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: serde::Deserializer<'de>
    {
        let s = String::deserialize(deserializer)?;
        Ok(Self(s.split(".").map(str::to_owned).collect()))
    }
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub enum RecordInner {
    SOA {
        serial: u32,
        mname: Name,
        rname: Name,
        refresh: u32,
        retry: u32,
        expire: u32,
        minimum: u32,
    },

    NS {
        ns: Name,
    },

    A {
        addr: [u8; 4],
    },

    AAAA {
        addr: [u8; 16],
    },

    CNAME {
        to: Name,
    },

    TXT {
        content: String,
    },
}

impl RecordInner {
    pub fn ty(&self) -> crate::parser::Type {
        use crate::parser::Type;
        use RecordInner::*;
        match self {
            SOA { .. } => Type::SOA,
            NS { .. } => Type::NS,
            A { .. } => Type::A,
            AAAA { .. } => Type::AAAA,
            CNAME { .. } => Type::CNAME,
            TXT { .. } => Type::TXT,
        }
    }

    pub fn serialize(&self) -> std::io::Result<Vec<u8>> {
        let mut ret = Vec::new();
        match self {
            RecordInner::SOA { serial, mname, rname, refresh, retry, expire, minimum } => {
                serialize_name(&mname.0, &mut ret)?;
                serialize_name(&rname.0, &mut ret)?;
                ret.write(&serial.to_be_bytes())?;
                ret.write(&refresh.to_be_bytes())?;
                ret.write(&retry.to_be_bytes())?;
                ret.write(&expire.to_be_bytes())?;
                ret.write(&minimum.to_be_bytes())?;
            },
            RecordInner::NS { ns } => { serialize_name(&ns.0, &mut ret)?; }
            RecordInner::A { addr } => { ret.write(addr)?; }
            RecordInner::AAAA { addr } => { ret.write(addr)?; }
            RecordInner::CNAME { to } => { serialize_name(&to.0, &mut ret)?; }
            RecordInner::TXT { content } => { ret.write(content.as_bytes())?; }
        }

        Ok(ret)
    }
}

#[derive(Deserialize, Debug)]
pub struct Record {
    #[serde(flatten)]
    pub inner: RecordInner,

    pub ttl: u32,
}

impl Record {
    pub fn serialize<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        // TYPE
        w.write_all(
            &(self.inner.ty() as u16).to_be_bytes()
        )?;

        // CLASS
        w.write_all(
            &[0, 1] // IN
        )?;

        // TTL
        w.write_all(
            &self.ttl.to_be_bytes()
        )?;

        let rdata = self.inner.serialize()?;

        // TODO: handles overflow
        w.write_all(&(rdata.len() as u16).to_be_bytes())?;
        w.write_all(&rdata)?;

        Ok(())
    }
}

pub fn serialize_name<W: Write>(segs: &[String], w: &mut W) -> std::io::Result<()> {
    for seg in segs.iter() {
        w.write_all(&[seg.len() as u8])?;
        w.write_all(seg.as_bytes())?;
    }

    w.write_all(&[0])?;

    Ok(())
}
