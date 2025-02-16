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
use std::io::{self, Read, Write};
use std::convert::TryInto;
use std::os::unix::fs::symlink;
use std::sync::atomic::{AtomicI8, Ordering};

const BUFFER_SIZE: usize = 2 << 13; // Chunk size for copying
const ARG_HELP: &str = "h";
const ARG_FORCE_REPLACE: &str = "f";

static G_APPLY_TO_ALL_ACTION: AtomicI8 = AtomicI8::new(0);
const APPLY_TO_ALL_ACTION_KEEP_BOTH: i8 = 1;
const APPLY_TO_ALL_ACTION_STOP: i8 = 2;
const APPLY_TO_ALL_ACTION_REPLACE: i8 = 3;

fn help() {
    let args: Vec<String> = env::args().collect();
    println!("usage: {} [ -{} ] <source path(s)> <destination path>", &args[0], ARG_HELP);
    println!();
    println!("  Copies all items in source to destination");
    println!();
    println!("  values:");
    println!("    <source path(s)> : relative or absolute");
    println!("    <destination path> : relative or absolute");
}

fn main() {
    let mut error = 0;
    let (h, srcs, dest) = read_arguments();

    if h {
        help();
    } else {
        error = copy_from_source_to_destination(&srcs, &dest);
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
fn read_arguments() -> (bool, Vec<String>, String) {
    let args: Vec<String> = env::args().collect();
    let mut help: bool = false;
    let mut src: Vec<String> = Vec::new();
    let mut dest: String = String::new();

    if args.len() < 2 {
        help = true;
    }

    for i in 1..args.len() {
        let arg = &args[i];

        // if this is a flag
        if (i == 1) && arg.starts_with("-") {
            if arg.contains(ARG_HELP) {
                help = true;
            } else if arg.contains(ARG_FORCE_REPLACE) {
                G_APPLY_TO_ALL_ACTION.store(
                    APPLY_TO_ALL_ACTION_REPLACE,
                    Ordering::Relaxed
                );
            }
        } else {
            if i < (args.len() - 1) {
                src.push(arg.clone());
            } else {
                dest = arg.clone();
            }
        }
    }

    return (help, src, dest);
}

/**
 * Copies all items in s to d
 */
fn copy_from_source_to_destination(s: &Vec<String>, d: &String) -> i32 {
    // vector of source/destination pairs
    let mut flows: Vec<FileFlow> = Vec::new();

    println!(" - Unfolding sources for all leaf items");
    print!(" - Items found: 0");

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
                    flows.push(FileFlow::new(&full_source_path, &file, &full_dest_path));
                }
            }
        }
    }

    // Get full paths for params

    println!();

    // Do copy
    // 
    // This will only copy one by one
    println!(" - Initiating copy...");
    let size = flows.len();
    for (i, flow) in flows.iter_mut().enumerate() {
        // make sure we know where we are copying to
        if flow.setup() != 0 {
            return -1;
        }

        // Execute copy
        match flow.copy(i + 1, size) {
            Ok(_) => {}
            Err(e) => {
                eprintln!(" ! Error copying file {}: {}", flow.source, e);
                return -1;
            }
        }
    }

    return 0;
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
        print!("\r - Items found: {}", *found_item_count);
        let p = Path::new(path);
        let abs_path = p.to_lexical_absolute().unwrap().to_str().unwrap().to_string();
        result.push(abs_path);
    } else if Path::new(path).is_file() {
        *found_item_count += 1;
        print!("\r - Items found: {}", *found_item_count);

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

struct FileFlow {
    /// Source file
    pub source: String,

    /// destination path
    pub destination: String,

    /// Base path where source is from
    base: String,

    /// Where source file will go respecting
    /// the file structure in base path
    new_destination: String
}

impl FileFlow {

    fn new(b: &String, s: &String, d: &String) -> Self {
        FileFlow {
            base: b.to_string(),
            source: s.to_string(),
            destination: d.to_string(),
            new_destination: String::new()
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

    /// Copies source to newDestination
    pub fn copy(&self, curr_index: usize, total_files: usize) -> io::Result<()> {
        let mut source_file = fs::File::open(&self.source)?;
        let permissions = source_file.metadata()?.permissions();

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
            let source_size: usize = source_file.metadata().unwrap().len().try_into().unwrap();
            let mut destination_file = fs::File::create(&self.new_destination)?;
            let mut buffer = [0; BUFFER_SIZE];
            let mut total_bytes_copied = 0;
            let file_name = Path::new(&self.new_destination).file_name().unwrap().to_str().unwrap();

            print!(" - ({} / {}) {} - {:.2}%",
                   curr_index, total_files,
                   file_name,
                   (total_bytes_copied as f64 / source_size as f64) * 100.0);
            loop {
                let bytes_read = source_file.read(&mut buffer)?;
                if bytes_read == 0 {
                    break; // End of file
                }

                destination_file.write_all(&buffer[..bytes_read])?;

                total_bytes_copied += bytes_read;

                print!("\r");
                print!(" - ({} / {}) {} - {:.2}%",
                       curr_index, total_files,
                       file_name,
                       (total_bytes_copied as f64 / source_size as f64) * 100.0);
            }
            println!("\r - ({} / {}) {} - {:.2}%", 
                     curr_index, total_files,
                     file_name,
                     (total_bytes_copied as f64 / source_size as f64) * 100.0);
            
            if let Err(e) = fs::set_permissions(Path::new(&self.new_destination), permissions) {
                eprintln!(" ! could not set permissions on '{}': {}", self.new_destination, e);
                return Err(e);
            }
        }

        Ok(())
    }
}

