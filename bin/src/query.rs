use trust_dns_proto::{
    op::{Message, MessageType, Query},
    rr::{Name, RecordType},
};

pub fn simple(id: u16, record: Name, qtype: RecordType) -> Message {
    let mut msg = Message::new();
    msg.set_id(id)
        .add_query(Query::query(record, qtype))
        .set_message_type(MessageType::Query);
    msg
}
