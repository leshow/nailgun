use std::{
    fs::File,
    io::{BufRead, BufReader, Lines},
    path::PathBuf,
    str::FromStr,
};

use anyhow::Result;
use trust_dns_proto::{
    op::{Message, MessageType, Query},
    rr::{DNSClass, Name, RecordType},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    File(PathBuf),
    Static {
        name: Name,
        qtype: RecordType,
        class: DNSClass,
    },
}

pub trait QueryGen {
    fn next_msg(&mut self, id: u16) -> Option<Message>;
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
    fn next_msg(&mut self, id: u16) -> Option<Message> {
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
            .set_message_type(MessageType::Query);
        Some(msg)
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
    fn next_msg(&mut self, id: u16) -> Option<Message> {
        let mut msg = Message::new();
        let mut query = Query::query(self.name.clone(), self.qtype);
        if self.class != DNSClass::IN {
            query.set_query_class(self.class);
        }
        msg.set_id(id)
            .add_query(query)
            .set_message_type(MessageType::Query);
        Some(msg)
    }
}

impl StaticGen {
    pub fn new(name: Name, qtype: RecordType, class: DNSClass) -> Self {
        Self { name, qtype, class }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomPkt {
    name: Name,
    qtype: RecordType,
}

impl QueryGen for RandomPkt {
    /// generate a simple query using a given id, record and qtype
    fn next_msg(&mut self, id: u16) -> Option<Message> {
        let mut msg = Message::new();
        msg.set_id(id)
            .add_query(Query::query(self.name.clone(), self.qtype))
            .set_message_type(MessageType::Query);
        Some(msg)
    }
}

impl RandomPkt {
    pub fn new(name: Name, qtype: RecordType) -> Self {
        Self { name, qtype }
    }
}
