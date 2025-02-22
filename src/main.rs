/**
 * author: Brando
 * date: 7/6/23
 *
 * https://doc.rust-lang.org/rust-by-example/error/result.html
 */

use std::env;
use std::any::type_name;
use std::fs;
use std::path::PathBuf;
use std::path::Path;
use std::path::Component;
use std::fs::canonicalize;
use std::io::{self, Read, Write, BufReader, BufWriter};
use std::os::unix::fs::symlink;
use std::sync::atomic::{AtomicI8, Ordering, AtomicU64};
use std::thread;
use std::time::{Duration, Instant};
use std::panic;
use std::sync::Arc;
use crossbeam_queue::ArrayQueue;
use progress_bar::*;
use std::collections::VecDeque;
use sha2::{Sha256, Digest};
use std::sync::Mutex;
use hex;

const VERSION_STRING: &str = "0.2";
const BUFFER_SIZE: u64 = 1024_u64.pow(2) * 10; // Chunk size for copying
const STREAM_SIZE: usize = 2 << 5;
const ARG_HELP: &str = "h";
const ARG_FORCE_REPLACE: &str = "f";
const ARG_VERSION: &str = "--version";
const ARG_CHECK: &str = "--check";

static G_APPLY_TO_ALL_ACTION: AtomicI8 = AtomicI8::new(0);
const APPLY_TO_ALL_ACTION_KEEP_BOTH: i8 = 1;
const APPLY_TO_ALL_ACTION_STOP: i8 = 2;
const APPLY_TO_ALL_ACTION_REPLACE: i8 = 3;

fn help() {
    let args: Vec<String> = env::args().collect();
    println!("usage: {} [ -{}{} ] [ <options> ] <source path(s)> <destination path>",
        &args[0],
        ARG_HELP,
        ARG_FORCE_REPLACE
    );
    println!();
    println!("  flags:");
    println!("    -h : prints help");
    println!("    -f : forces replacements");
    println!();
    println!("  options:");
    println!("    --version : prints version");
    println!("    --check : certifies if source was completely copied to destination");
    println!();
    println!("  arguments:");
    println!("    <source path(s)> : relative or absolute");
    println!("    <destination path> : relative or absolute");
    println!();
    println!("version {}, Copyright © 2024 Brando. All rights reserved.", VERSION_STRING);
}

fn print_version() {
    println!("{}", VERSION_STRING);
}

fn main() {
    let mut error = 0;
    let (h, show_version, check, srcs, dest) = read_arguments();
    if h {
        help();
    } else if show_version {
        print_version();
    } else {
        error = copy_from_source_to_destination(&srcs, &dest, check);
    }

    std::process::exit(error);
}

/**
 * reads arguments
 *
 * return (
 * 0 : help
 * 1 : source paths
 * 2 : destination
 * )
 */
fn read_arguments() -> (bool, bool, bool, Vec<String>, String) {
    let args: Vec<String> = env::args().collect();
    let mut help: bool = false;
    let mut version: bool = false;
    let mut check: bool = false;
    let mut src: Vec<String> = Vec::new();
    let mut dest: String = String::new();

    if args.len() < 2 {
        help = true;
    }

    for i in 1..args.len() {
        let arg = &args[i];

        // if this is a flag
        if (i == 1) && arg.starts_with("-")  && !arg.starts_with("--") {
            if arg.contains(ARG_HELP) {
                help = true;
                continue;
            } else if arg.contains(ARG_FORCE_REPLACE) {
                G_APPLY_TO_ALL_ACTION.store(
                    APPLY_TO_ALL_ACTION_REPLACE,
                    Ordering::Relaxed
                );
                continue;
            }
        } 
        
        if arg.contains(ARG_CHECK) {
            check = true;
        } else if arg.contains(ARG_VERSION) {
            version = true;
        } else if i < (args.len() - 1) {
            src.push(arg.clone());
        } else {
            dest = arg.clone();
        }
    }

    return (help, version, check, src, dest);
}

trait LexicalAbsolute {
    fn to_lexical_absolute(&self) -> io::Result<PathBuf>;
}

impl LexicalAbsolute for Path {
    fn to_lexical_absolute(&self) -> std::io::Result<PathBuf> {
        let mut absolute = if self.is_absolute() {
            PathBuf::new()
        } else {
            std::env::current_dir()?
        };
        for component in self.components() {
            match component {
                Component::CurDir => {},
                Component::ParentDir => { absolute.pop(); },
                component @ _ => absolute.push(component.as_os_str()),
            }
        }
        Ok(absolute)
    }
}

/**
 * Finds all file paths within path
 */
fn find_leaf_files(path: &str, found_item_count: &mut i32) -> Result<Vec<String>, std::io::Error> {
    let mut result = Vec::new();

    if Path::new(path).is_symlink() {
        *found_item_count += 1;
        print!("\rItems found: {}", *found_item_count);
        let p = Path::new(path);
        let abs_path = p.to_lexical_absolute().unwrap().to_str().unwrap().to_string();
        result.push(abs_path);
    } else if Path::new(path).is_file() {
        *found_item_count += 1;
        print!("\rItems found: {}", *found_item_count);

        let expanded_path = canonicalize(path).unwrap().into_os_string().into_string().unwrap();
        result.push(expanded_path.to_owned());
    } else { // else directory
        let entries = fs::read_dir(path)?; // Read directory entries
        
        for entry in entries {
            let entry = entry?;
            let subdir_files = find_leaf_files(entry.path().to_str().unwrap(), found_item_count)?;
            result.extend(subdir_files);
        }
    }
    
    Ok(result)
}

/**
 * takes in a path and checks if this path already exists, if so the result will be
 * an altered name that doesn't have conflict
 *
 * Function will also prompt the user for actions like keeping duplicates, stopping operation,
 * or replacing the duplicate.
 */
fn filter_for_conflicts(path: PathBuf) -> Result<PathBuf, i32> {
    let mut result: PathBuf = path.clone();
    if !result.exists() {
        return Ok(result);
    } else {
        let mut ans = String::new();
        let val: i8 = G_APPLY_TO_ALL_ACTION.load(Ordering::Relaxed);
        if val != 0 {
            ans = val.to_string();
        } else {
            println!(" ! {} already exists. Would you like to...", result.display());
            println!(" ! {}: Keep Both", APPLY_TO_ALL_ACTION_KEEP_BOTH);
            println!(" ! {}: Stop", APPLY_TO_ALL_ACTION_STOP);
            println!(" ! {}: Replace", APPLY_TO_ALL_ACTION_REPLACE);
            print!(" ! >> ");
            io::stdout().flush().unwrap();
        
            io::stdin().read_line(&mut ans).expect("failed to readline");
            ans = ans.trim_matches('\n').to_string();

            print!(" ! Apply to all? [y/n] >> ");
            io::stdout().flush().unwrap();
            let mut ans2 = String::new();
            io::stdin().read_line(&mut ans2).expect("failed to readline");
            ans2 = ans2.trim_matches('\n').to_string();
            if ans2 == "y" {
                let val: i8 = ans.parse().expect("Failed to parse string to integer");
                G_APPLY_TO_ALL_ACTION.store(val, Ordering::Relaxed);
            }
        }
        
        match ans.parse().expect("Failed to parse string to integer") {
            APPLY_TO_ALL_ACTION_KEEP_BOTH => { }
            APPLY_TO_ALL_ACTION_STOP => {
                eprintln!(" - stopping...");
                return Err(1);
            }
            APPLY_TO_ALL_ACTION_REPLACE => {
                return Ok(result);
            },
            i8::MIN..=0_i8 | 4_i8..=i8::MAX => panic!(" ! unkown response: {}", ans),
        }
    }

    let extension = path.extension();
    let Some(basename) = path.file_stem() else {
        eprintln!(" ! couldn't get base name (w/o ext) from '{}'", path.display());
        return Err(1);
    };

    let mut i = 1;
    while result.exists() {
        result.pop();
        match extension {
            Some(ext) => {
                result.push(format!("{}_{}.{}", basename.to_str().unwrap(), i, ext.to_str().unwrap()));
            }
            None => {
                result.push(format!("{}_{}", basename.to_str().unwrap(), i));
            }
        }

        i += 1;
    }
    return Ok(result);
}

/**
 * Copies all items in s to d
 *
 * certify: certifies copy
 */
fn copy_from_source_to_destination(s: &Vec<String>, d: &String, certify: bool) -> i32 {
    // vector of source/destination pairs
    let mut flows: VecDeque<FileFlow> = VecDeque::new();
    let overall_elapsed_time = Instant::now();

    print!("Items found: 0");
    init_progress_bar(0);

    let mut counter = 0;
    let full_dest_path = canonicalize(d).unwrap().into_os_string().into_string().unwrap();
    for source in s.iter() {
        let full_source_path = canonicalize(source).unwrap().into_os_string().into_string().unwrap();
    
        // Find all items in source directory
        match find_leaf_files(source, &mut counter) {
            Err(e) => {
                eprintln!(" ! Experienced an error in: {} - {}", type_name::<fn()>(), e.kind()); 
                return -1;
            } Ok(files) => {
                for file in files {
                    flows.push_back(FileFlow::new(&full_source_path, &file, &full_dest_path));
                }
            }
        }
    }

    // Get full paths for params

    println!();

    // Do copy
    // 
    // This will only copy one by one
    let num_flows = flows.len();
    let mut i = 0;
    while let Some(mut flow) = flows.pop_front() {
        // make sure we know where we are copying to
        if flow.setup() != 0 {
            return -1;
        }

        let source_size = flow.source_size();
        let file_name = flow.new_destination_file_name();

        init_progress_bar(source_size as usize);
        enable_eta();
        print_progress_bar_info(
            "Item",
            format!("({} / {}) '{}'", i + 1, num_flows, file_name).as_str(),
            Color::Green,
            Style::Bold
        );
        set_progress_bar_width(25);

        // Execute copy
        let flow_time = Instant::now();
        let dest_size = Arc::new(AtomicU64::new(0));
        let arc_dest_size = Arc::clone(&dest_size);
        if let Err(e) = flow.copy(certify, move |destination_size| {
            arc_dest_size.store(destination_size, Ordering::Relaxed);
        }) {
            eprintln!(" ! Error copying file {}: {}", flow.source, e);
            return -1;
        }

        // wait for copy to end
        set_progress_bar_action("Copying", Color::Cyan, Style::Bold);
        loop {
            let dsize = dest_size.load(Ordering::Relaxed);
            if source_size <= dsize {
                break;
            } else {
                set_progress_bar_progress(dsize as usize);
            }
            thread::sleep(Duration::from_millis(5));
        }
        print_progress_bar_info(
            "Copied",
            format!("'{}' in {} seconds, {} bytes", file_name, flow_time.elapsed().as_secs(), source_size).as_str(),
            Color::Green,
            Style::Bold
        );

        // read destination
        if certify {
            let flow_time = Instant::now();
            dest_size.store(0, Ordering::Relaxed);
            let arc_dest_size = Arc::clone(&dest_size);
            if let Err(e) = flow.read_destination(move |destination_size| {
                arc_dest_size.store(destination_size, Ordering::Relaxed);
            }) {
                eprintln!(" ! Error reading file {}: {}", flow.destination, e);
                return -1;
            }

            // wait for check to end
            set_progress_bar_action("Reading", Color::Cyan, Style::Bold);
            loop {
                let dsize = dest_size.load(Ordering::Relaxed);
                if source_size <= dsize {
                    break;
                } else {
                    set_progress_bar_progress(dsize as usize);
                }
                thread::sleep(Duration::from_millis(5));
            }
            print_progress_bar_info(
                "Read",
                format!("'{}' in {} seconds, {} bytes", file_name, flow_time.elapsed().as_secs(), source_size).as_str(),
                Color::Green,
                Style::Bold
            );

            match flow.check() {
                Ok(res) => {
                    print_progress_bar_info(
                        "Success",
                        format!("SHA-256('{}')", res).as_str(),
                        Color::Green,
                        Style::Bold
                    );
                }
                Err((s,d)) => {
                    print_progress_bar_info(
                        "Source",
                        format!("source hash: SHA-256('{}')", s).as_str(),
                        Color::Yellow,
                        Style::Bold
                    );

                    print_progress_bar_info(
                        "Failure",
                        format!("incorrect hash: SHA-256('{}')", d).as_str(),
                        Color::Red,
                        Style::Bold
                    );
                }
            }
        }

        flow.set_permissions();

        drop(flow);
        i += 1;
    }
    print_progress_bar_final_info(
        "Finished",
        format!("copied all in {} seconds", overall_elapsed_time.elapsed().as_secs()).as_str(),
        Color::Green,
        Style::Bold
    );
    finalize_progress_bar();

    return 0;
}

struct FileFlow {
    /// Source file
    pub source: String,

    /// destination path
    pub destination: String,

    /// Base path where source is from
    base: String,

    /// Where source file will go respecting
    /// the file structure in base path
    new_destination: String,

    hash_source: Arc<Mutex<Sha256>>,
    hash_dest: Arc<Mutex<Sha256>>,
}

impl FileFlow {

    fn new(b: &String, s: &String, d: &String) -> Self {
        FileFlow {
            base: b.to_string(),
            source: s.to_string(),
            destination: d.to_string(),
            new_destination: String::new(),
            hash_source: Arc::new(Mutex::new(Sha256::new())),
            hash_dest: Arc::new(Mutex::new(Sha256::new())),
        }
    }

    /**
     * returns the source file's size
     */
    fn source_size(&self) -> u64 {
        let Ok(file) = fs::File::open(&self.source) else {
            return 0
        };

        let Ok(md) = file.metadata() else {
            return 0
        };

        md.len()
    }

    /**
     * returns the leaf item of the new destination path
     *
     * new_destination is specified in setup()
     */
    fn new_destination_file_name(&self) -> String {
        let default_name: &str = "<unknown file name...>";
        let file_name = Path::new(&self.new_destination).file_name();
        if file_name.is_none() {
            return String::from(default_name)
        }

        match file_name.unwrap().to_os_string().into_string() {
            Ok(res) => {
                res
            }
            Err(_) => {
                String::from(default_name)
            }
        }
    }

    /**
     * compares the hashes of the source and destination
     *
     * if they match then copy was successful
     *
     * Ok() will return the destination hash. 
     * Err(s, h) returns both hashes (source, destination). This will mean they are incorrect
     */
    fn check(&self) -> Result<String, (String, String)> {
        let shash = &self.hash_source.lock().unwrap().clone().finalize().to_vec();
        let dhash = &self.hash_dest.lock().unwrap().clone().finalize().to_vec();
        if shash == dhash {
            Ok(hex::encode(dhash))
        } else {
            Err((
                hex::encode(shash),
                hex::encode(dhash)
            ))
        }
    }

    /**
     * Returns the relative leaf path from base
     *
     * if base == source, then file_name() of source is returned
     */
    fn source_rel_leaf(&self) -> String {
        let mut result = PathBuf::new();
 
        // if source == base then we can assume:
        //
        // 1: source input is a file, we just need to append file name to destination path
        // 2: source input is an empty directory. This case is not handled yet
        if self.source == self.base {
            let source_path = Path::new(&self.source);
            if let Some(leaf) = source_path.file_name() {
                result.push(leaf);
            }
        } else {
            result.push(Path::new(&self.base).file_name().unwrap());
            
            // strip base from source path
            let mut leaf_rel_path = self.source.replace(&self.base, "");

            // remove the "/" so it is not treated as an absolute path but
            // rather a relative path
            leaf_rel_path = Path::new(&leaf_rel_path).strip_prefix("/").unwrap().display().to_string();

            // We use the leaf component of base to add to the destination path
            // because we want to make sure if the input source to this
            // program is a directory, we make sure we copy from the root of 
            // the source
            result.push(leaf_rel_path);
        }

        return result.into_os_string().into_string().unwrap();
    }

    /// sets newDestination
    pub fn setup(&mut self) -> i32 {
        let mut dest_path = PathBuf::from(&self.destination);
        
        // Create new destination path, keeping the structure of the
        // base path
        dest_path.push(self.source_rel_leaf());

        // check for conflicts
        let Ok(dest_path) = filter_for_conflicts(dest_path) else {
            eprintln!(" ! error filtering for destination path");
            return 1;
        };
        
        self.new_destination = dest_path.clone().into_os_string().into_string().unwrap();

        // Make sure sub directories are created
        let dest_parent_path = dest_path.parent().unwrap();
        match fs::create_dir_all(dest_parent_path) {
            Ok(_) => {}
            Err(e) => {
                eprintln!(" ! could not create directory {}: {}", dest_parent_path.display(), e);
                return -1;
            }
        }

        return 0;
    }

    /**
     * Copies source to newDestination
     *
     * update_callback: gets invoked for every buffer that is written to destination. value is the
     * new size of the destination
     *
     * make_source_hash: makes sha2-256 hash using the sha2 lib
     */
    pub fn copy<F: Fn(u64) + Send + 'static>(&self, make_source_hash: bool, update_callback: F) -> io::Result<()> {
        let source_file = fs::File::open(&self.source)?;
        let destination_file = fs::File::create(&self.new_destination)?;

        if Path::new(&self.source).is_symlink() {
            let target = fs::read_link(&self.source)?;
            // Get the relative target path
            let relative_target = if target.is_absolute() {
                let dst_parent = Path::new(&self.new_destination).parent().unwrap(); // Assuming parent directory always exists
                dst_parent.join(target.strip_prefix("/").unwrap())
            } else {
                target
            };

            // Create a new symbolic link at the destination with the relative target
            match symlink(&relative_target, &self.new_destination) {
                Err(e) => eprintln!(" ! couldn't make a symbolic link for {}: {}", relative_target.display(), e),
                Ok(_) => {
                    println!(" - symbolic link created: {}", self.new_destination);
                }
            }
        } else {
            let source_size = self.source_size();
            let stack_size = BUFFER_SIZE as usize * STREAM_SIZE as usize;
            let stream: Arc<ArrayQueue<[u8;BUFFER_SIZE as usize]>> = Arc::new(ArrayQueue::new(STREAM_SIZE));

            // read thread
            let stream_rh = Arc::clone(&stream);
            let hash_source_rh: Option<Arc<Mutex<Sha256>>> = if make_source_hash {
                Some(Arc::clone(&self.hash_source))
            } else {
                None
            };
            thread::Builder::new()
                .name("read_thread".into())
                .stack_size(stack_size)
                .spawn(move || {
                    let mut total_read = 0;
                    let mut readbuf = BufReader::with_capacity(BUFFER_SIZE as usize, source_file);
                    loop {
                        if !stream_rh.is_full() {
                            let buf_size: u64 = if (source_size - total_read) > BUFFER_SIZE {
                                BUFFER_SIZE
                            } else {
                                source_size - total_read
                            };
                            let mut buffer = [0; BUFFER_SIZE as usize];
                            match readbuf.read(&mut buffer[..buf_size as usize]) {
                                Ok(bytes_read) => {
                                    if bytes_read > 0 {
                                        total_read += bytes_read as u64;
                                        if let Err(_) = stream_rh.push(buffer) {
                                            eprintln!(" ! couldn't send buf to write thread");
                                            break;
                                        }
                                        if let Some(hash) = &hash_source_rh {
                                            hash.lock().unwrap().update(&buffer);
                                        }
                                    } else {
                                        break; // End of file
                                    }
                                }
                                Err(e) => {
                                    panic!("{}", e);
                                }
                            }
                        }
                    }
            }).unwrap();

            // write thread
            let stream_wh = Arc::clone(&stream);
            thread::Builder::new()
                .name("write_thread".into())
                .stack_size(stack_size)
                .spawn(move || {
                    let mut writebuf = BufWriter::with_capacity(BUFFER_SIZE as usize, destination_file);
                    let mut total_bytes_copied = 0;
                    let mut wait_count = 0;
                    while total_bytes_copied < source_size {
                        if stream_wh.is_empty() {
                            wait_count += 1;
                            if wait_count > (2 << 12) {
                                thread::sleep(Duration::from_nanos(5));
                            } else if wait_count > (2 << 8) {
                                thread::sleep(Duration::from_nanos(1));
                            }
                            continue;
                        }
                        wait_count = 0;
                        let buffer = stream_wh.pop().unwrap();
                        let buf_size: u64 = if (source_size - total_bytes_copied) > BUFFER_SIZE {
                            BUFFER_SIZE
                        } else {
                            source_size - total_bytes_copied
                        };
                        if let Err(e) = writebuf.write_all(&buffer[..buf_size as usize]) {
                            panic!("{}", e);
                        }
                        total_bytes_copied += buf_size;
                        update_callback(total_bytes_copied);
                    }
            }).unwrap();
        }

        Ok(())
    }

    /**
     * calculates the sha has for destination. Uses update_callback to update caller on the 
     * read progress
     */
    pub fn read_destination<F: Fn(u64) + Send + 'static>(&self, update_callback: F) -> io::Result<()> {
        let destination_file = fs::File::open(&self.new_destination)?;
        let source_size = self.source_size();
        let stack_size = BUFFER_SIZE as usize * STREAM_SIZE as usize;

        // read thread
        let hash_dest_rh = Arc::clone(&self.hash_dest);
        thread::Builder::new()
            .name("read_thread".into())
            .stack_size(stack_size)
            .spawn(move || {
                let mut total_read = 0;
                let mut readbuf = BufReader::with_capacity(BUFFER_SIZE as usize, destination_file);
                loop {
                    let buf_size: u64 = if (source_size - total_read) > BUFFER_SIZE {
                        BUFFER_SIZE
                    } else {
                        source_size - total_read
                    };
                    let mut buffer = [0; BUFFER_SIZE as usize];
                    match readbuf.read(&mut buffer[..buf_size as usize]) {
                        Ok(bytes_read) => {
                            if bytes_read > 0 {
                                total_read += bytes_read as u64;
                                hash_dest_rh.lock().unwrap().update(&buffer);
                                update_callback(total_read);
                            } else {
                                break; // End of file
                            }
                        }
                        Err(e) => {
                            panic!("read error {}", e);
                        }
                    }
                }
        }).unwrap();

        Ok(())
    }

    /**
     * copies the source's permissions to the destination
     */
    fn set_permissions(&self) {
        let Ok(source_file) = fs::File::open(&self.source) else {
            eprintln!(" ! could not open file");
            return;
        };

        let Ok(md) = source_file.metadata() else {
            eprintln!(" ! could not get metadata"); return;
        };

        if let Err(e) = fs::set_permissions(Path::new(&self.new_destination), md.permissions()) {
            eprintln!(" ! could not set permissions on '{}': {}", self.new_destination, e);
        }
    }
}

