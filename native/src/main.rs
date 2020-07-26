// TODO(kvakil): this is all one big file. split it.
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::convert::TryInto;
use std::fs;
use std::fs::File;
use std::io;
use std::io::{Read, Seek, Write};
use std::iter::FromIterator;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
extern crate byteorder;
use byteorder::NativeEndian;
use byteorder::ReadBytesExt;
extern crate serde;
extern crate serde_json;
extern crate xdg;

#[macro_use]
extern crate serde_derive;

use itertools::Itertools;
use rustc_hash::{FxHashMap, FxHashSet};

// TODO(kvakil): check sync version between this and Talon script for IPC?
const VERSION: &str = "v0";

type DocumentId = usize;
type Document = (DocumentId, String);
type Word = String;
type Score = f32;
type ScoreResult = FxHashMap<DocumentId, Score>;

#[derive(Debug)]
struct WordIndex {
    frequency_by_document: FxHashMap<DocumentId, f32>,
}

#[derive(Debug)]
struct TfidfIndex {
    number_of_documents: usize,
    document_bags: FxHashMap<DocumentId, FxHashSet<String>>,
    word_indices: FxHashMap<Word, WordIndex>,
}

// TODO(kvakil): we should remove all uses us this function.
fn ignore<T>(_: T) -> () {}

// TODO(kvakil): make this impl TfidfIndex.
fn score(tfidf_index: &TfidfIndex, query: String) -> ScoreResult {
    let mut scores = FxHashMap::<DocumentId, Score>::default();
    // TODO(kvakil): do we want .unique()?
    query.split_whitespace().unique().for_each(|word| {
        let word_index = tfidf_index.word_indices.get(word);
        match word_index {
            None => return,
            Some(word_index) => {
                // TODO(kvakil): better weighing
                let idf = ((1 + tfidf_index.number_of_documents) as f32
                    / ((1 + word_index.frequency_by_document.len()) as f32))
                    .ln();
                word_index
                    .frequency_by_document
                    .iter()
                    .for_each(|(&document_id, &tf)| {
                        let score = scores.entry(document_id).or_insert(0.0);
                        *score += tf * idf;
                    });
            }
        }
    });
    return scores;
}

// TODO(kvakil): make this impl TfidfIndex.
fn update_index(tfidf_index: &mut TfidfIndex, document_id: DocumentId, document_content: &String) {
    // TODO(kvakil): better token stream. We can split inside a word, and translate numbers.
    let words: Vec<String> = document_content
        .split(|ch: char| !ch.is_ascii_alphabetic())
        .map(|word| word.to_ascii_lowercase())
        .collect();
    let inverse_document_length: f32 = (words.len() as f32).recip();
    remove_from_index(tfidf_index, document_id);
    let bag_of_words = tfidf_index
        .document_bags
        .entry(document_id)
        .or_insert_with(|| FxHashSet::<String>::default());
    for word in words {
        let word_index = tfidf_index
            .word_indices
            .entry(word.to_string())
            .or_insert_with(|| WordIndex {
                frequency_by_document: FxHashMap::<DocumentId, f32>::default(),
            });
        let frequency_in_document = (*word_index)
            .frequency_by_document
            .entry(document_id)
            .or_insert(0.0);
        *frequency_in_document += inverse_document_length;
        bag_of_words.insert(word.to_string());
    }
    tfidf_index.number_of_documents += 1
}

// TODO(kvakil): make this impl TfidfIndex.
fn remove_from_index(tfidf_index: &mut TfidfIndex, document_id: DocumentId) -> Option<()> {
    // TODO(kvakil): we slowly leak memory, because if a word no longer has any documents
    // associated with it, it still has an entry in the word_indices map. OTOH, it will
    // probably be added in update_index. Perhaps a background task to compress "unused"
    // entries? (Or likely, this is all too complicated, and we can live with the leak.
    tfidf_index
        .document_bags
        .remove_entry(&document_id)
        .and_then(|(_, bag)| {
            tfidf_index.number_of_documents -= 1;
            bag.iter().for_each(|word| {
                tfidf_index
                    .word_indices
                    .entry(word.to_string())
                    .and_modify(|word_index| {
                        word_index.frequency_by_document.remove_entry(&document_id);
                    });
            });
            Some(())
        })
}

// TODO(kvakil): make this impl TfidfIndex.
fn get_words_in_index(tfidf_index: &TfidfIndex) -> FxHashSet<String> {
    let mut s = FxHashSet::<String>::default();
    // TODO(kvakil): remove clone
    for (_, bag) in tfidf_index.document_bags.clone() {
        for word in bag {
            s.insert(word);
        }
    }
    s
}

// TODO(kvakil): make this impl TfidfIndex.
fn make_index(documents: Vec<Document>) -> TfidfIndex {
    let mut tfidf_index = TfidfIndex {
        number_of_documents: 0,
        word_indices: FxHashMap::<Word, WordIndex>::default(),
        document_bags: FxHashMap::<DocumentId, FxHashSet<String>>::default(),
    };
    // TODO(kvakil): tear this out? I don't think anyone needs the documents argument.
    documents
        .iter()
        .for_each(|(document_id, document)| update_index(&mut tfidf_index, *document_id, document));
    return tfidf_index;
}

type TabId = u64;

enum MessageToParentThread {
    MessageFromBrowser(MessageFromBrowser),
    MessageFromWorkerThread(MessageFromWorkerThread),
    MessageFromTalonThread(MessageFromTalonThread),
}

#[derive(Serialize, Deserialize)]
enum MessageFromBrowser {
    FocusTab {
        #[serde(rename = "tabId")]
        tab_id: TabId,
    },
    UpdateIndex {
        #[serde(rename = "tabId")]
        tab_id: TabId,
        updated: Vec<Document>,
        removed: Vec<DocumentId>,
    },
    CloseTab {
        #[serde(rename = "tabId")]
        tab_id: TabId,
    },
}

enum MessageToWorkerThread {
    FocusTab {},
    UpdateIndex {
        updated: Vec<Document>,
        removed: Vec<DocumentId>,
    },
    Query {
        query: String,
    },
    CloseTab {},
}

enum MessageFromWorkerThread {
    Score {
        tab_id: TabId,
        scores: ScoreResult,
    },
    UpdateTalonRequest {
        tab_id: TabId,
        words: FxHashSet<String>,
    },
}

enum MessageToOutputThread {
    Score { tab_id: TabId, scores: ScoreResult },
}

#[derive(Serialize, Deserialize)]
enum MessageFromTalonThread {
    Query {
        #[serde(rename = "tabId")]
        tab_id: TabId,
        query: String,
    },
}

#[derive(Serialize, Deserialize)]
enum MessageToTalonThread {
    UpdateTalonRequest {
        #[serde(rename = "tabId")]
        tab_id: TabId,
        words: FxHashSet<String>,
    },
}

struct Thread {
    input: mpsc::Sender<MessageToWorkerThread>,
}

type WorkerThreads = FxHashMap<TabId, Thread>;

fn spawn_worker_thread(
    parent_thread_tx: &mpsc::Sender<MessageToParentThread>,
    tab_id: TabId,
) -> Thread {
    let tx = parent_thread_tx.clone();
    let (txp, rxp) = mpsc::channel();
    thread::spawn(move || {
        let mut tfidf_index = make_index(vec![]);
        loop {
            match rxp.recv() {
                Ok(MessageToWorkerThread::FocusTab {}) => {
                    tx.send(MessageToParentThread::MessageFromWorkerThread(
                        MessageFromWorkerThread::UpdateTalonRequest {
                            tab_id,
                            words: get_words_in_index(&tfidf_index),
                        },
                    ));
                }
                Ok(MessageToWorkerThread::UpdateIndex { updated, removed }) => {
                    updated
                        .iter()
                        .for_each(|(id, doc)| update_index(&mut tfidf_index, *id, doc));

                    removed.iter().for_each(|id| {
                        ignore::<Option<()>>(remove_from_index(&mut tfidf_index, *id))
                    });

                    tx.send(MessageToParentThread::MessageFromWorkerThread(
                        MessageFromWorkerThread::UpdateTalonRequest {
                            tab_id,
                            words: get_words_in_index(&tfidf_index),
                        },
                    ));
                }
                Ok(MessageToWorkerThread::Query { query }) => {
                    let scores = MessageToParentThread::MessageFromWorkerThread(
                        MessageFromWorkerThread::Score {
                            tab_id,
                            scores: score(&tfidf_index, query),
                        },
                    );
                    tx.send(scores).unwrap()
                }
                Ok(MessageToWorkerThread::CloseTab {}) => break,
                Err(_) => break,
            }
        }
    });
    return Thread { input: txp };
}

fn get_or_spawn_thread<'a>(
    worker_threads: &'a mut WorkerThreads,
    tab_id: TabId,
    parent_thread_tx: &'a mpsc::Sender<MessageToParentThread>,
) -> &'a mut Thread {
    return worker_threads
        .entry(tab_id)
        .or_insert_with(|| spawn_worker_thread(&parent_thread_tx, tab_id));
}

fn spawn_parent_thread(
    output_thread_tx: mpsc::Sender<MessageToOutputThread>,
    talon_thread_tx: mpsc::Sender<MessageToTalonThread>,
) -> mpsc::Sender<MessageToParentThread> {
    let (parent_thread_tx, parent_thread_rx) = mpsc::channel::<MessageToParentThread>();
    let parent_thread_tx_for_return = parent_thread_tx.clone();
    let mut threads = FxHashMap::<TabId, Thread>::default();
    thread::spawn(move || loop {
        let message = parent_thread_rx.recv().unwrap();
        // TODO(kvakil): remove all the ignores here, gracefully[?] handle errors.
        // Parent thread should probably stay around even if a child dies.
        match message {
            MessageToParentThread::MessageFromBrowser(MessageFromBrowser::FocusTab { tab_id }) => {
                ignore(
                    get_or_spawn_thread(&mut threads, tab_id, &parent_thread_tx)
                        .input
                        .send(MessageToWorkerThread::FocusTab {}),
                )
            }
            MessageToParentThread::MessageFromTalonThread(MessageFromTalonThread::Query {
                query,
                tab_id,
            }) => ignore(
                get_or_spawn_thread(&mut threads, tab_id, &parent_thread_tx)
                    .input
                    .send(MessageToWorkerThread::Query { query }),
            ),
            MessageToParentThread::MessageFromWorkerThread(MessageFromWorkerThread::Score {
                tab_id,
                scores,
            }) => ignore(output_thread_tx.send(MessageToOutputThread::Score { tab_id, scores })),
            MessageToParentThread::MessageFromBrowser(MessageFromBrowser::UpdateIndex {
                tab_id,
                updated,
                removed,
            }) => ignore(
                get_or_spawn_thread(&mut threads, tab_id, &parent_thread_tx)
                    .input
                    .send(MessageToWorkerThread::UpdateIndex { updated, removed }),
            ),
            // TODO(kvakil): only update Talon if this is actually the active tab.
            MessageToParentThread::MessageFromWorkerThread(
                MessageFromWorkerThread::UpdateTalonRequest { tab_id, words },
            ) => ignore(
                talon_thread_tx.send(MessageToTalonThread::UpdateTalonRequest { tab_id, words }),
            ),
            MessageToParentThread::MessageFromBrowser(MessageFromBrowser::CloseTab { tab_id }) => {
                ignore(
                    get_or_spawn_thread(&mut threads, tab_id, &parent_thread_tx)
                        .input
                        .send(MessageToWorkerThread::CloseTab {}),
                )
            }
        }
    });

    return parent_thread_tx_for_return;
}

#[derive(Serialize, Deserialize)]
struct MessageToBrowser {
    #[serde(rename = "tabId")]
    tab_id: TabId,
    best: Vec<DocumentId>,
}

use serde::Serialize;
use std::error::Error;
fn dump<S: Serialize>(s: S) -> Result<(), Box<dyn Error>> {
    let message = serde_json::to_string(&s)?;
    let size = u32::to_ne_bytes(message.len().try_into().unwrap());
    let mut out = std::io::stdout();
    out.write_all(&size)?;
    out.write_all(message.as_bytes())?;
    out.flush()?;
    Ok(())
}

fn read_browser_message(input: &mut io::StdinLock) -> std::vec::Vec<u8> {
    let length = input.read_u32::<NativeEndian>().unwrap();
    let mut message = input.take(length as u64);
    let mut buffer = Vec::with_capacity(length as usize);
    message.read_to_end(&mut buffer);
    return buffer;
}

// TODO(kvakil): split this function up.
fn main() {
    let (talon_thread_tx, talon_thread_rx) = mpsc::channel::<MessageToTalonThread>();
    let (output_thread_tx, output_thread_rx) = mpsc::channel::<MessageToOutputThread>();
    let parent_thread_tx = spawn_parent_thread(output_thread_tx, talon_thread_tx);
    let xdg_dirs = xdg::BaseDirectories::with_prefix("voicesurf").unwrap();
    let talon_input_directory = xdg_dirs.create_runtime_directory("input").unwrap();
    let talon_preinput_directory = xdg_dirs.create_runtime_directory("preinput").unwrap();

    // Talon updating thread
    thread::spawn(move || {
        let talon_input_filename = talon_input_directory.join(VERSION);
        let talon_preinput_filename = talon_preinput_directory.join(VERSION);
        let talon_input_path = talon_input_filename.as_path();
        let talon_preinput_path = talon_preinput_filename.as_path();
        let mut talon_preinput_file = File::create(talon_preinput_path).unwrap();
        loop {
            match talon_thread_rx.recv() {
                Ok(message) => {
                    talon_preinput_file.seek(io::SeekFrom::Start(0)).unwrap();
                    talon_preinput_file.set_len(0).unwrap();
                    talon_preinput_file
                        .write_all(serde_json::to_string(&message).unwrap().as_bytes())
                        .unwrap();
                    talon_preinput_file.sync_all().unwrap();
                    fs::copy(talon_preinput_path, talon_input_path).unwrap();
                }
                Err(_) => break,
            }
        }
    });

    // Talon receiving thread
    let (talon_receive_tx, talon_receive_rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new_raw(talon_receive_tx).unwrap();
    let talon_output_directory = xdg_dirs.create_runtime_directory("output").unwrap();
    let talon_output_filename = talon_output_directory.join(VERSION);
    let parent_thread_tx_for_talon = parent_thread_tx.clone();
    watcher
        .watch(talon_output_directory, RecursiveMode::NonRecursive)
        .unwrap();
    thread::spawn(move || {
        let talon_output_path = talon_output_filename.as_path();
        loop {
            match talon_receive_rx.recv() {
                // TODO(kvakil): scope this event?
                Ok(_event) => {
                    let mut talon_output_file = File::open(talon_output_path).unwrap();
                    let mut buffer = String::new();
                    talon_output_file.read_to_string(&mut buffer).unwrap();
                    let message: MessageFromTalonThread =
                        serde_json::from_slice(buffer.as_bytes()).unwrap();
                    parent_thread_tx_for_talon
                        .send(MessageToParentThread::MessageFromTalonThread(message))
                        .unwrap();
                }
                Err(_) => break,
            }
        }
    });

    // Output thread
    thread::spawn(move || loop {
        match output_thread_rx.recv() {
            Ok(MessageToOutputThread::Score { tab_id, scores }) => {
                let mut best_by_score = scores.into_iter().collect::<Vec<(DocumentId, Score)>>();
                // TODO(kvakil): this can be more efficient -- we don't need the whole sort
                // obviously
                best_by_score.sort_by(|(doc_id0, score0), (doc_id1, score1)| {
                    score1.partial_cmp(score0).unwrap_or(doc_id0.cmp(doc_id1))
                });
                // Send top 10 arbitrarily.
                // TODO(kvakil): maybe just send all, or all above a threshold, or only send
                // the few which are "far better"?
                // TODO(kvakil): maybe make a floor here, so we don't send anything if all the
                // choices are really bad.
                best_by_score.truncate(10);
                // TODO(kvakil): structured logging?
                eprintln!("output: dumping to browser");
                dump(MessageToBrowser {
                    tab_id,
                    best: best_by_score.iter().map(|(id, _score)| *id).collect(),
                })
                .unwrap();
            }
            Err(_) => break,
        }
    });

    // Input thread
    let stdin = io::stdin();
    let mut input = stdin.lock();
    loop {
        eprintln!("input: reading...");
        let buffer = read_browser_message(&mut input);
        eprintln!("input: got message...");
        let message: MessageFromBrowser = serde_json::from_slice(&buffer).unwrap();
        match parent_thread_tx.send(MessageToParentThread::MessageFromBrowser(message)) {
            Ok(()) => (),
            Err(_) => break,
        }
    }
}

// TODO(kvakil): more tests.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_index_and_query() {
        let documents: Vec<String> = vec![
            String::from("this is sample").to_string(),
            String::from("this is another another example example example").to_string(),
        ];

        let tfidf_index: TfidfIndex = make_index(
            documents
                .iter()
                .enumerate()
                .map(|(id, doc)| (id, doc.to_string()))
                .collect(),
        );

        let scores_this_is = score(&tfidf_index, "this is".to_string());
        assert_eq!(scores_this_is.get(&0).cloned().unwrap_or(0.0), 0.0);
        assert_eq!(scores_this_is.get(&1).cloned().unwrap_or(0.0), 0.0);

        let scores_example = score(&tfidf_index, "example".to_string());
        assert_eq!(scores_example.get(&0).cloned().unwrap_or(0.0), 0.0);
        assert_eq!(scores_example.get(&1).cloned().unwrap_or(0.0), 0.17377077);
    }

    #[test]
    fn create_index_and_update() {
        let documents: Vec<String> = vec![
            String::from("this is sample").to_string(),
            String::from("this will get overwritten").to_string(),
        ];

        let mut tfidf_index: TfidfIndex = make_index(
            documents
                .iter()
                .enumerate()
                .map(|(id, doc)| (id, doc.to_string()))
                .collect(),
        );

        update_index(
            &mut tfidf_index,
            1,
            &String::from("will be overwrriten this is").to_string(),
        );
        update_index(
            &mut tfidf_index,
            1,
            &String::from("will be overwrriten this is example").to_string(),
        );
        update_index(
            &mut tfidf_index,
            1,
            &String::from("this is another another example example example").to_string(),
        );

        let scores_this_is = score(&tfidf_index, "this is".to_string());
        assert_eq!(scores_this_is.get(&0).cloned().unwrap_or(0.0), 0.0);
        assert_eq!(scores_this_is.get(&1).cloned().unwrap_or(0.0), 0.0);

        let scores_example = score(&tfidf_index, "example".to_string());
        assert_eq!(scores_example.get(&0).cloned().unwrap_or(0.0), 0.0);
        assert_eq!(scores_example.get(&1).cloned().unwrap_or(0.0), 0.17377077);
    }

    #[test]
    fn create_index_and_query_threaded() {
        let documents_data: Vec<String> = vec![
            String::from("this is sample").to_string(),
            String::from("this is another another example example example").to_string(),
        ];
        let documents = documents_data
            .iter()
            .enumerate()
            .map(|(id, doc)| (id, doc.to_string()))
            .collect();
        let tab_id = 3;
        let (tx, rx) = mpsc::channel();
        let (txp, rxp) = mpsc::channel();
        let parent_thread_tx = spawn_parent_thread(tx, txp);
        parent_thread_tx
            .send(MessageToParentThread::MessageFromBrowser(
                MessageFromBrowser::FocusTab { tab_id },
            ))
            .unwrap();
        parent_thread_tx
            .send(MessageToParentThread::MessageFromBrowser(
                MessageFromBrowser::UpdateIndex {
                    tab_id,
                    updated: documents,
                    removed: vec![],
                },
            ))
            .unwrap();
        parent_thread_tx
            .send(MessageToParentThread::MessageFromTalonThread(
                MessageFromTalonThread::Query {
                    tab_id,
                    query: "example".to_string(),
                },
            ))
            .unwrap();
        match rx.recv() {
            Ok(MessageToOutputThread::Score {
                scores: scores_example,
                tab_id: _,
            }) => {
                assert_eq!(scores_example.get(&0).cloned().unwrap_or(0.0), 0.0);
                assert_eq!(scores_example.get(&1).cloned().unwrap_or(0.0), 0.17377077);
            }
            Err(_) => assert!(false),
        }
    }
}
