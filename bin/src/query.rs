use std::{
    fs::File,
    io::{BufRead, BufReader, Lines},
    path::PathBuf,
    str::FromStr,
};

use anyhow::Result;
use rand::Rng;
use tracing::error;
use trust_dns_proto::{
    op::{Edns, Message, MessageType, Query},
    rr::{rdata::opt::EdnsCode, DNSClass, Name, RecordType},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    File(PathBuf),
    Static {
        name: Name,
        qtype: RecordType,
        class: DNSClass,
    },
    RandomPkt {
        size: usize,
    },
    RandomQName {
        qtype: RecordType,
        size: usize,
    },
}

pub trait QueryGen {
    fn next_msg(&mut self, id: u16) -> Option<Vec<u8>>;
}

#[derive(Debug)]
pub struct FileGen {
    rdr: Lines<BufReader<File>>,
    // could reuse buf so we don't allocate a new string for each Message
    // buf: String,
}

impl FileGen {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let rdr = BufReader::new(File::open(path)?).lines();
        Ok(Self { rdr })
    }
}

impl QueryGen for FileGen {
    // TODO: do we just exit when we run out of things to send?
    fn next_msg(&mut self, id: u16) -> Option<Vec<u8>> {
        let line = self
            .rdr
            .next()?
            .expect("FileGen encountered an error reading line");
        let mut next = line.split_ascii_whitespace();
        let name = Name::from_ascii(next.next()?)
            .expect("FileGen encountered an error parsing Name from line");
        let qtype = RecordType::from_str(next.next()?)
            .expect("FileGen encountered an error parsing RecordType from line");
        let mut msg = Message::new();
        msg.set_id(id)
            .add_query(Query::query(name, qtype))
            .set_message_type(MessageType::Query)
            .set_recursion_desired(true)
            .set_edns(Edns::new());

        match msg.to_vec() {
            Ok(msg) => Some(msg),
            Err(err) => {
                error!(?err);
                None
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticGen {
    name: Name,
    qtype: RecordType,
    class: DNSClass,
}

impl QueryGen for StaticGen {
    /// generate a simple query using a given id, record and qtype
    fn next_msg(&mut self, id: u16) -> Option<Vec<u8>> {
        let mut msg = Message::new();
        let mut query = Query::query(self.name.clone(), self.qtype);
        if self.class != DNSClass::IN {
            query.set_query_class(self.class);
        }
        msg.set_id(id)
            .add_query(query)
            .set_message_type(MessageType::Query)
            .set_recursion_desired(true)
            .set_edns(Edns::new());
        match msg.to_vec() {
            Ok(msg) => Some(msg),
            Err(err) => {
                error!(?err);
                None
            }
        }
    }
}

impl StaticGen {
    pub fn new(name: Name, qtype: RecordType, class: DNSClass) -> Self {
        Self { name, qtype, class }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomPkt {
    len: usize,
}

impl QueryGen for RandomPkt {
    /// generate a random message
    fn next_msg(&mut self, id: u16) -> Option<Vec<u8>> {
        let mut msg = id.to_be_bytes().to_vec();
        msg.extend((0..self.len).map(|_| rand::random::<u8>()));
        Some(msg)
    }
}

impl RandomPkt {
    pub fn new(len: usize) -> Self {
        Self { len }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomQName {
    len: usize,
    qtype: RecordType,
}

impl QueryGen for RandomQName {
    /// generate a random label only
    fn next_msg(&mut self, id: u16) -> Option<Vec<u8>> {
        use rand::distributions::Alphanumeric;
        let gen_label = |len| {
            let rng = rand::thread_rng();
            rng.sample_iter(Alphanumeric)
                .map(char::from)
                .take(len)
                .collect::<String>()
        };

        let qname = match Name::from_str(&gen_label(self.len)) {
            Ok(qname) => Some(qname),
            Err(err) => {
                error!(?err);
                None
            }
        }?;

        let mut msg = Message::new();
        msg.set_id(id)
            .add_query(Query::query(qname, self.qtype))
            .set_message_type(MessageType::Query)
            .set_recursion_desired(true)
            .set_edns(Edns::new());

        match msg.to_vec() {
            Ok(msg) => Some(msg),
            Err(err) => {
                error!(?err);
                None
            }
        }
    }
}

impl RandomQName {
    pub fn new(qtype: RecordType, len: usize) -> Self {
        Self { qtype, len }
    }
}
