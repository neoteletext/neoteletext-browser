//! What the worker does with the files it is given: the run itself, from
//! the command line passed with them to the files it gives back. The
//! worker process calls it confined; a run that cannot be confined (the
//! live network, `--unconfined`) calls it in the unconfined process; the
//! tests call it directly. It opens no file.

use crate::args::{self, NetworkMode, Options};
use crate::blobs::{Blob, find};
use crate::live::Live;
use crate::network::{Network, Recorder, Refusing, Table};
use crate::run::{MESSAGE, STATUS, run};
use crate::{Failure, Status, files};

/// The network a run's options ask for. The live modes start the real
/// web client, which no confined process can: they are for the runner's
/// unconfined process alone.
///
/// # Errors
/// Status 4: a network file the reader refuses, or a web client that
/// cannot be started.
pub fn network_for(options: &Options, given: &[Blob]) -> Result<Box<dyn Network>, Failure> {
    let live = || Live::start(&options.channel_url, options.serve.is_some());
    match &options.network {
        NetworkMode::None => Ok(Box::new(Refusing::default())),
        NetworkMode::Mock(_) | NetworkMode::Replay(_) => {
            let bytes = find(given, files::NETWORK).ok_or_else(|| Failure::usage("the network file was not given to the worker"))?;
            let entries = crate::network::parse(bytes).map_err(Failure::usage)?;
            Ok(Box::new(Table::new(entries, matches!(options.network, NetworkMode::Replay(_)))))
        }
        NetworkMode::Live => Ok(Box::new(live()?)),
        NetworkMode::Record(_) => Ok(Box::new(Recorder::new(Box::new(live()?)))),
    }
}

/// A run's files, or a failure's, as what the worker writes back.
fn answer(result: Result<Vec<Blob>, Failure>, status: Status) -> Vec<Blob> {
    match result {
        Ok(mut files) => {
            files.push((STATUS.to_owned(), vec![b'0'.saturating_add(status.code())]));
            files
        }
        Err(failure) => vec![(STATUS.to_owned(), vec![b'0'.saturating_add(failure.status.code())]), (MESSAGE.to_owned(), failure.message.into_bytes())],
    }
}

/// Runs on files already read: what the worker does.
#[must_use]
pub fn run_given(given: &[Blob]) -> Vec<Blob> {
    let made = (|| {
        let arguments = find(given, files::ARGUMENTS)
            .and_then(|bytes| core::str::from_utf8(bytes).ok())
            .ok_or_else(|| Failure::usage("no arguments were given to the worker"))?;
        let arguments: Vec<String> = arguments.split('\0').map(str::to_owned).collect();
        let mut options = args::parse(&arguments)?;
        // A folder served from this computer is the channel: its address
        // is the server's.
        if let Some(folder) = &options.serve {
            options.channel_url = crate::live::serve(folder)?;
        }
        let network = network_for(&options, given)?;
        run(&options, given, network)
    })();
    match made {
        Ok(outcome) => {
            let status = outcome.report.status;
            let mut files = outcome.files;
            if !outcome.report.message.is_empty() {
                files.push((MESSAGE.to_owned(), outcome.report.message.into_bytes()));
            }
            answer(Ok(files), status)
        }
        Err(failure) => answer(Err(failure), Status::Usage),
    }
}
