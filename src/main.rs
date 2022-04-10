mod parser;
mod record;

use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use log::debug;
use log::info;
use parser::ReqHeaderStatus;
use structopt::StructOpt;
use tokio::net::UdpSocket;

use crate::record::serialize_name;
use crate::record::Name;

#[derive(StructOpt)]
struct Args {
    #[structopt(short, long, default_value = "53")]
    port: u16,

    #[structopt(short, long, default_value = "0.0.0.0")]
    host: String,

    #[structopt(short, long, default_value = "base.yml")]
    base: PathBuf,
}

type BaseStorage = HashMap<Name, Vec<record::Record>>;
struct RecordStorage {
    pub base: BaseStorage,
}

impl RecordStorage {
    pub fn query_all<'a>(
        &'a self,
        segs: &[String],
    ) -> impl Iterator<Item = &'a record::Record> + 'a {
        let base = self.base.get(segs);
        let ret: Box<dyn Iterator<Item = &record::Record>> = match base {
            Some(b) => Box::new(b.iter()),
            None => Box::new(std::iter::empty()),
        };
        ret
    }

    pub fn query<'a>(
        &self,
        segs: &'a [String],
        ty: parser::Type,
    ) -> (&'a [String], Vec<&record::Record>) {
        let mut collected = Vec::new();

        for item in self.query_all(segs) {
            if item.inner.ty() == ty {
                collected.push(item);
            }
        }

        if collected.len() == 0 && ty.need_recursive() && segs.len() > 1 {
            self.query(&segs[1..], ty)
        } else {
            (segs, collected)
        }
    }
}

#[repr(u8)]
#[allow(dead_code)]
enum Rcode {
    OK = 0,
    Format = 1,
    Internal = 2,
    Name = 3,
    NotImpl = 4,
    Refused = 5,
}

fn write_resp_header<W: Write>(
    writer: &mut W,
    id: u16,
    rcode: Rcode,
    is_aa: bool,
    req_status: &ReqHeaderStatus,

    cnts: [u16; 4],
) -> anyhow::Result<()> {
    writer.write_all(&id.to_be_bytes())?;
    writer.write_all(&[
        0x80 // QR(1 = R)
        | (req_status.opcode as u8) << 3
        | (if is_aa { 1 << 2 } else { 0 }) // AA
        | req_status.rd as u8,
        rcode as u8,
    ])?;

    for cnt in cnts {
        writer.write_all(&cnt.to_be_bytes())?;
    }
    Ok(())
}

async fn handle(
    buf: Vec<u8>,
    socket: Arc<UdpSocket>,
    remote: SocketAddr,
    storage: Arc<RecordStorage>,
) -> anyhow::Result<()> {
    debug!("Recieved from {}", remote);
    debug!("{:?}", buf);

    let mut output_buffer = Vec::new();

    let parsed = match parser::parse(buf.as_slice()) {
        Ok((_, parsed)) => parsed,
        Err(e) => {
            log::error!("Malformed request: {}", e);
            if buf.len() < 4 {
                return Ok(());
            }
            let id = u16::from_be_bytes([buf[0], buf[1]]);
            let hdr_status = if let Ok((_, st)) = parser::parse_header_status(&buf[2..]) {
                st
            } else {
                return Ok(());
            };
            write_resp_header(
                &mut output_buffer,
                id,
                Rcode::Format,
                true,
                &hdr_status,
                [0, 0, 0, 0],
            )?;
            socket.send_to(&output_buffer, &remote).await?;
            return Ok(());
        }
    };

    log::debug!("Request: {:?}", parsed);

    if parsed.questions.len() != 1 {
        log::error!("Unimplemented: query with \\neq 1 question");

        write_resp_header(
            &mut output_buffer,
            parsed.header.id,
            Rcode::NotImpl,
            true,
            &parsed.header.status,
            [0, 0, 0, 0],
        )?;
        socket.send_to(&output_buffer, &remote).await?;
        return Ok(());
    }

    let q = &parsed.questions[0];
    if q.name.ptr.is_some() {
        log::error!("Unimplemented: query with ptr in name");

        write_resp_header(
            &mut output_buffer,
            parsed.header.id,
            Rcode::NotImpl,
            true,
            &parsed.header.status,
            [0, 0, 0, 0],
        )?;
        socket.send_to(&output_buffer, &remote).await?;
        return Ok(());
    }

    let segs: Vec<String> = q
        .name
        .labels
        .iter()
        .map(|seg| seg.clone().into_owned())
        .collect();
    let (mut scope, mut answers) = storage.query(&segs, q.ty);

    // Check self CNAME
    if answers.len() == 0 && q.ty != parser::Type::CNAME && q.ty != parser::Type::NS {
        (scope, answers) = storage.query(&segs, parser::Type::CNAME);
    }

    // For all recursive requests, additionally check is a nearer NS is present
    if answers.len() > 0 && q.ty.need_recursive() && q.ty != parser::Type::NS {
        let (nsscope, nsanswers) = storage.query(&segs, parser::Type::NS);
        if nsscope.len() > scope.len() {
            scope = nsscope;
            answers = nsanswers;
        }
    }

    // Finally, nothing is found. Check authoritative servers
    if answers.len() == 0 && q.ty != parser::Type::NS {
        (scope, answers) = storage.query(&segs, parser::Type::NS);
    }

    log::debug!("Answers @ {:?}: {:#?}", scope, answers);

    let rcode = if answers.len() > 0 {
        Rcode::OK
    } else {
        Rcode::Name
    };

    let is_ns = answers.len() > 0 && answers[0].inner.ty() == parser::Type::NS;

    write_resp_header(
        &mut output_buffer,
        parsed.header.id,
        rcode,
        !is_ns,
        &parsed.header.status,
        [
            0, // TODO: Copy questions
            if is_ns { 0 } else { answers.len() as u16 },
            if !is_ns { 0 } else { answers.len() as u16 },
            0,
        ],
    )?;

    for answer in answers {
        serialize_name(scope, &mut output_buffer)?;
        answer.serialize(&mut output_buffer)?;
    }

    socket.send_to(&output_buffer, &remote).await?;
    Ok(())
}

#[paw::main]
#[tokio::main]
async fn main(args: Args) -> anyhow::Result<()> {
    env_logger::init();
    info!("Listening on {}:{}...", args.host, args.port);
    let socket = Arc::new(UdpSocket::bind((args.host, args.port)).await?);
    debug!("Socket open");

    let base_file = std::fs::File::open(&args.base)?;
    let base: BaseStorage = serde_yaml::from_reader(base_file)?;
    debug!("Base: {:#?}", base);

    let storage = Arc::new(RecordStorage { base });

    loop {
        let mut buf = vec![0; 65536];
        let (len, remote) = socket.recv_from(&mut buf).await?;
        buf.resize(len, 0);

        tokio::spawn(handle(buf, socket.clone(), remote, storage.clone()));
    }
}
