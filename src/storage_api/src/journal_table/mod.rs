use crate::{GetFuture, GetStream, PutFuture};
use restate_common::types::{EntryIndex, JournalEntry, ServiceId};

pub trait JournalTable {
    fn put_journal_entry(
        &mut self,
        service_id: &ServiceId,
        journal_index: u32,
        journal_entry: JournalEntry,
    ) -> PutFuture;

    fn get_journal_entry(
        &mut self,
        service_id: &ServiceId,
        journal_index: u32,
    ) -> GetFuture<Option<JournalEntry>>;

    fn get_journal(
        &mut self,
        service_id: &ServiceId,
        journal_length: EntryIndex,
    ) -> GetStream<JournalEntry>;

    fn delete_journal(&mut self, service_id: &ServiceId, journal_length: EntryIndex) -> PutFuture;
}
