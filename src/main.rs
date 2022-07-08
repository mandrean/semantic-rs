extern crate clap;
extern crate clog;
extern crate env_logger;
extern crate git2;
extern crate hubcaps;
#[macro_use]
extern crate log;
extern crate regex;
extern crate semver;
extern crate tokio;
extern crate toml;
extern crate url;

use std::io::Write;
use std::path::Path;
use std::process::exit;
use std::thread;
use std::time::Duration;
use std::{env, fs};

use clap::{App, Arg, ArgMatches};
use env_logger::{fmt::Color, Builder, Env};
use semver::Version;

use crate::commit_analyzer::CommitType;
use crate::config::ConfigBuilder;
use crate::utils::user_repo_from_url;

mod cargo;
mod changelog;
mod commit_analyzer;
mod config;
mod error;
mod git;
mod github;
mod preflight;
mod toml_file;
mod utils;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const USERAGENT: &str = concat!("semantic-rs/", env!("CARGO_PKG_VERSION"));

const COMMITTER_ERROR_MESSAGE: &str = r"
A release commit needs a committer name and email address.
We tried fetching it from different locations, but couldn't find one.

Committer information is taken from the following environment variables, if set:

GIT_COMMITTER_NAME
GIT_COMMITTER_EMAIL

If none is set the normal git config is tried in the following order:

Local repository config
User config
Global config";

macro_rules! info_exit {
    ($fmt:expr) => {{
        info!($fmt);
        exit(0);
    }};
    ($fmt:expr, $($arg:tt)*) => {{
        info!($fmt, $($arg)*);
        exit(0);
    }};
}

macro_rules! error_exit {
    ($fmt:expr) => {{
        error!($fmt);
        exit(1);
    }};
    ($fmt:expr, $($arg:tt)*) => {{
        error!($fmt, $($arg)*);
        exit(1);
    }};
}

fn string_to_bool(answer: &str) -> bool {
    matches!(&answer.to_lowercase()[..], "yes" | "true" | "1")
}

fn version_bump(version: &Version, bump: CommitType) -> Option<Version> {
    let mut version = version.clone();

    // NB: According to the Semver spec, major version zero is for
    // the initial development phase is treated slightly differently.
    // The minor version is incremented for breaking changes
    // and major is kept at zero until the public API has become more stable.
    if version.major == 0 {
        match bump {
            CommitType::Unknown => return None,
            CommitType::Patch => version.patch += 1,
            CommitType::Minor => version.patch += 1,
            CommitType::Major => version.minor += 1,
        }
    } else {
        match bump {
            CommitType::Unknown => return None,
            CommitType::Patch => version.patch += 1,
            CommitType::Minor => version.minor += 1,
            CommitType::Major => version.major += 1,
        }
    }

    Some(version)
}

#[test]
fn test_breaking_bump_major_zero() {
    let buggy_release = Version::parse("0.2.0").unwrap();
    let bumped_version = version_bump(&buggy_release, CommitType::Major).unwrap();
    assert_eq!(bumped_version, Version::parse("0.3.0").unwrap());
}

#[test]
fn test_breaking_bump_major_one() {
    let buggy_release = Version::parse("1.0.0").unwrap();
    let bumped_version = version_bump(&buggy_release, CommitType::Major).unwrap();
    assert_eq!(bumped_version, Version::parse("2.0.0").unwrap());
}

fn ci_env_set() -> bool {
    env::var("CI").is_ok()
}

fn current_branch(repo: &git2::Repository) -> Option<String> {
    let head = repo.head().expect("No HEAD found for repository");

    if head.is_branch() {
        let short = head.shorthand().expect("No branch name found");
        return Some(short.into());
    }

    None
}

fn is_release_branch(current: &str, release: &str) -> bool {
    current == release
}

fn push_to_github(config: &config::Config, tag_name: &str) {
    info!("Pushing new commit and tag");
    git::push(&config, &tag_name)
        .unwrap_or_else(|err| error_exit!("Failed to push git: {:?}", err));

    info!("Waiting a tiny bit, so GitHub can store the git tag");
    thread::sleep(Duration::from_secs(1));
}

fn release_on_github(config: &config::Config, tag_message: &str, tag_name: &str) {
    if github::can_release(&config) {
        info!("Creating GitHub release");
        github::release(&config, &tag_name, &tag_message)
            .unwrap_or_else(|err| error_exit!("Failed to create GitHub release: {:?}", err));
    } else {
        info!("Project not hosted on GitHub. Skipping release step");
    }
}

fn release_on_cratesio(config: &config::Config) {
    info!("Publishing crate on crates.io");
    if !cargo::publish(
        &config.repository_path,
        &config.cargo_token.as_ref().unwrap(),
    ) {
        error_exit!("Failed to publish on crates.io");
    }
}

fn generate_changelog(repository_path: &str, version: &Version, new_version: &str) -> String {
    info!("New version would be: {}", new_version);
    info!("Would write the following Changelog:");
    match changelog::generate(repository_path, &version.to_string(), new_version) {
        Ok(_log) => _log,
        Err(err) => {
            error_exit!("Generating Changelog failed: {:?}", err);
        }
    }
}

fn write_changelog(repository_path: &str, version: &Version, new_version: &str) {
    info!("Writing Changelog");
    changelog::write(repository_path, &version.to_string(), &new_version)
        .unwrap_or_else(|err| error!("Writing Changelog failed: {:?}", err));
}

fn print_changelog(changelog: &str) {
    info!("====================================");
    info!("{}", changelog);
    info!("====================================");
    info!("Would create annotated git tag");
}

fn package_crate(config: &config::Config, repository_path: &str, new_version: &str) {
    if config.release_mode {
        info!("Updating lockfile");
        if !cargo::update_lockfile(repository_path) {
            error!("`cargo fetch` failed. See above for the cargo error message.");
        }
    }

    git::commit_files(&config, &new_version)
        .unwrap_or_else(|err| error!("Committing files failed: {:?}", err));

    info!("Package crate");
    if !cargo::package(repository_path) {
        error!("`cargo package` failed. See above for the cargo error message.");
    }
}

fn get_repo(repository_path: &str) -> git2::Repository {
    match git2::Repository::open(repository_path) {
        Ok(repo) => repo,
        Err(e) => {
            error_exit!("Could not open the git repository: {:?}", e);
        }
    }
}

fn get_repository_path(matches: &ArgMatches) -> String {
    let path = Path::new(matches.value_of("path").unwrap_or("."));
    let path = fs::canonicalize(path).unwrap_or_else(|_| {
        error_exit!("Path does not exist or a component is not a directory");
    });
    let repo_path = path.to_str().unwrap_or_else(|| {
        error_exit!("Path is not valid unicode");
    });
    repo_path.to_string()
}

fn get_signature<'a>(repository_path: String) -> git2::Signature<'a> {
    let repo = get_repo(&repository_path);
    let signature = match git::get_signature(&repo) {
        Ok(sig) => sig,
        Err(e) => {
            error!(
                "Failed to get the committer's name and email address: {}",
                e.to_string()
            );
            error_exit!("{}", COMMITTER_ERROR_MESSAGE);
        }
    };

    signature.to_owned()
}

fn get_user_and_repo(repository_path: &str) -> Option<(String, String)> {
    let repo = get_repo(repository_path);
    let remote_or_none = repo.find_remote("origin");
    match remote_or_none {
        Ok(remote) => {
            let url = remote
                .url()
                .expect("Remote URL is not valid UTF-8")
                .to_owned();
            let (user, repo_name) = user_repo_from_url(&url).unwrap_or_else(|e| {
                error_exit!(
                    "Could not extract user and repository name from URL: {:?}",
                    e
                );
            });

            Some((user, repo_name))
        }
        Err(err) => {
            warn!("Could not determine the origin remote url: {:?}", err);
            warn!("semantic-rs can't push changes or create a release on GitHub");
            None
        }
    }
}

fn get_github_creds(repository_path: &str) -> (Option<String>, Option<String>) {
    let repo = get_repo(repository_path);
    let remote_or_none = repo.find_remote("origin");
    match remote_or_none {
        Ok(remote) => {
            let url = remote
                .url()
                .expect("Remote URL is not valid UTF-8")
                .to_owned();
            if github::is_github_url(&url) {
                (env::var("GH_USERNAME").ok(), env::var("GH_TOKEN").ok())
            } else {
                (None, None)
            }
        }
        Err(_) => (None, None),
    }
}

fn get_cargo_token() -> Option<String> {
    env::var("CARGO_TOKEN").ok()
}

fn assemble_configuration(args: ArgMatches) -> config::Config {
    let mut config_builder = ConfigBuilder::new();

    // If write mode is requested OR denied,
    // adhere to the user's wish,
    // otherwise we decide based on whether we are running in CI.
    let write_mode = match args.value_of("write") {
        Some(write_mode) => string_to_bool(write_mode),
        None => ci_env_set(),
    };

    let release_flag = match args.value_of("release") {
        Some(release_mode) => string_to_bool(release_mode),
        None => false,
    };

    // We can only release, if we are allowed to write
    let release_mode = write_mode && release_flag;
    let repository_path = get_repository_path(&args);

    config_builder.write(write_mode);
    config_builder.release(release_mode);
    config_builder.branch(args.value_of("branch").unwrap_or("master").to_string());
    config_builder.repository_path(repository_path.clone());
    config_builder.signature(get_signature(repository_path.clone()));
    if let Some((user, repo)) = get_user_and_repo(&repository_path) {
        config_builder.user(user);
        config_builder.repository_name(repo);
    }
    if let (Some(gh_username), Some(gh_token)) = get_github_creds(&repository_path) {
        config_builder.gh_username(gh_username);
        config_builder.gh_token(gh_token);
    }
    if let Some(cargo_token) = get_cargo_token() {
        config_builder.cargo_token(cargo_token);
    }
    let repo = get_repo(&repository_path);
    match repo.find_remote("origin") {
        Ok(r) => config_builder.remote(Ok(r.name().unwrap().to_string())),
        Err(err) => config_builder.remote(Err(err.to_string())),
    };

    config_builder.repository(repo);
    config_builder.build()
}

fn init_logger() {
    let env = Env::default()
        .default_filter_or(log::Level::Info.to_string())
        .default_write_style_or("auto");

    Builder::from_env(env)
        .format(|buf, record| {
            let mut style = buf.style();
            match record.level() {
                log::Level::Info => writeln!(buf, "{}", record.args()),
                log::Level::Warn => writeln!(buf, "{}: {}", record.level(), record.args()),
                log::Level::Error => {
                    style.set_color(Color::Red).set_bold(true);
                    writeln!(buf, "{}: {}", style.value(record.level()), record.args())
                }
                _ => {
                    style.set_color(Color::Yellow).set_bold(true);
                    writeln!(buf, "{}: {}", style.value(record.level()), record.args())
                }
            }
        })
        .init();
}

fn main() {
    init_logger();
    info!("semantic.rs 🚀");

    let clap_args =  App::new("semantic-rs")
        .version(VERSION)
        .author("Fork by Sebastian Mandrean <sebastian.mandrean@gmail.com>")
        .about("Crate publishing done right")
        .arg(Arg::with_name("write")
             .short("w")
             .long("write")
             .help("Write changes to files (default: yes if CI is set, otherwise no).")
             .value_name("WRITE_MODE")
             .takes_value(true))
        .arg(Arg::with_name("release")
            .short("r")
            .long("release")
            .help("Create release on GitHub and publish on crates.io (only in write mode) [default: yes].")
            .value_name("RELEASE_MODE")
            .takes_value(true))
        .arg(Arg::with_name("branch")
             .short("b")
             .long("branch")
             .help("The branch on which releases should happen. [default: master].")
             .value_name("BRANCH")
             .takes_value(true))
        .arg(Arg::with_name("path")
             .short("p")
             .long("path")
             .help("Specifies the repository path. [default: .]")
             .value_name("PATH")
             .takes_value(true))
        .get_matches();

    let config = assemble_configuration(clap_args);

    let branch = current_branch(&config.repository).unwrap_or_else(|| {
        error_exit!("Could not determine current branch.");
    });

    if !is_release_branch(&branch, &config.branch) {
        info!(
            "Current branch is '{}', releases are only done from branch '{}'",
            branch, config.branch
        );
        info_exit!("No release done from a pull request either.");
    }

    //Before we actually start, we do perform some preflight checks
    //Here we check if everything is in place to do a GitHub release and a
    //release on crates.io.
    //The important bit is, if something's missing, we do not abort since the user can still do all
    //other things except publishing

    info!("Performing preflight checks now");
    let warnings = preflight::check(&config);

    if warnings.is_empty() {
        info!("Checks done. Everything is ok");
    }

    for warning in warnings {
        warn!("{}", warning);
    }

    let version = toml_file::read_from_file(&config.repository_path).unwrap_or_else(|err| {
        error_exit!("Reading `Cargo.toml` failed: {:?}", err);
    });

    let version = Version::parse(&version).expect("Not a valid version");
    info!("Current version: {}", version.to_string());

    info!("Analyzing commits");

    let bump = git::version_bump_since_latest(&config.repository);
    if config.write_mode {
        info!("Commits analyzed. Bump will be {:?}", bump);
    } else {
        info!("Commits analyzed. Bump would be {:?}", bump);
    }
    let new_version = match version_bump(&version, bump) {
        Some(new_version) => new_version.to_string(),
        None => {
            info_exit!("No version bump. Nothing to do.");
        }
    };

    if !config.write_mode {
        let changelog = generate_changelog(&config.repository_path, &version, &new_version);
        print_changelog(&changelog);
    } else {
        info!("New version: {}", new_version);

        toml_file::write_new_version(&config.repository_path, &new_version)
            .unwrap_or_else(|err| error!("Writing `Cargo.toml` failed: {:?}", err));

        write_changelog(&config.repository_path, &version, &new_version);
        package_crate(&config, &config.repository_path, &new_version);

        info!("Creating annotated git tag");
        let tag_message =
            changelog::generate(&config.repository_path, &version.to_string(), &new_version)
                .unwrap_or_else(|err| {
                    error_exit!("Can't generate changelog: {:?}", err);
                });

        let tag_name = format!("v{}", new_version);
        git::tag(&config, &tag_name, &tag_message)
            .unwrap_or_else(|err| error!("Failed to create git tag: {:?}", err));

        if config.release_mode && config.can_push() {
            push_to_github(&config, &tag_name);
        }

        if config.release_mode && config.can_release_to_github() {
            release_on_github(&config, &tag_message, &tag_name);
        }

        if config.release_mode && config.can_release_to_cratesio() {
            release_on_cratesio(&config);
            info!(
                "{} v{} is released. 🚀🚀🚀",
                config.repository_name.unwrap(),
                new_version
            );
        }
    }
}
