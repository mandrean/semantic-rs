use serde::Serialize;

use crate::config::Config;
use crate::error::Error;

use super::USERAGENT;

#[derive(Serialize)]
struct CreateRelease<'a> {
    tag_name: &'a str,
    name: &'a str,
    body: &'a str,
    target_commitish: &'a str,
    draft: bool,
    prerelease: bool,
}

pub fn can_release(config: &Config) -> bool {
    let repo = &config.repository;
    match repo.find_remote("origin") {
        Ok(remote) => {
            let url = match remote.url() {
                Some(u) => u,
                None => return false,
            };
            is_github_url(url)
        }
        Err(_) => false,
    }
}

pub fn is_github_url(url: &str) -> bool {
    url.contains("github.com")
}

pub fn release(config: &Config, tag_name: &str, tag_message: &str) -> Result<(), Error> {
    let user = config.user.as_ref().unwrap();
    let repo_name = config.repository_name.as_ref().unwrap();
    let token = config.gh_token.as_ref().unwrap();

    let url = format!(
        "https://api.github.com/repos/{}/{}/releases",
        user, repo_name
    );

    let body = CreateRelease {
        tag_name,
        name: tag_name,
        body: tag_message,
        target_commitish: &config.branch,
        draft: false,
        prerelease: false,
    };

    tokio::runtime::Runtime::new()
        .expect("Failed to create Tokio runtime")
        .block_on(async {
            reqwest::Client::builder()
                .user_agent(USERAGENT)
                .build()?
                .post(&url)
                .header("Authorization", format!("token {}", token))
                .header("Accept", "application/vnd.github.v3+json")
                .json(&body)
                .send()
                .await?
                .error_for_status()?;
            Ok::<(), reqwest::Error>(())
        })
        .map_err(Error::from)
}
