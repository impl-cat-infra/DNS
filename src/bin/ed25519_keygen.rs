use std::{path::PathBuf, os::unix::prelude::OpenOptionsExt, io::Write};

use rand::rngs::OsRng;
use structopt::StructOpt;

use ed25519::pkcs8::EncodePrivateKey;

#[derive(StructOpt)]
struct Args {
    #[structopt(short, long)]
    output: PathBuf,
}

#[paw::main]
fn main(args: Args) -> anyhow::Result<()> {
    env_logger::init();
    let mut file = std::fs::File::options().write(true).create(true).truncate(true).mode(0o600).open(&args.output)?;

    let mut osrng = OsRng;
    let kp = ed25519_dalek::Keypair::generate(&mut osrng);
    let enc_kp = ed25519::pkcs8::KeypairBytes::from_bytes(&kp.to_bytes());
    match enc_kp.to_pkcs8_pem(pem_rfc7468::LineEnding::LF) {
        Err(e) => {
            log::error!("Error: {}", e);
            return Err(anyhow::anyhow!("Failed to generate keypair."));
        }
        Ok(s) => file.write_all(s.as_bytes())?,
    }

    Ok(())
}