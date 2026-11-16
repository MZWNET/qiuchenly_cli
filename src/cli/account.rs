use anyhow::{bail, Result};
use std::io;

use crate::store::account;

pub async fn login(username: &str, password_stdin: bool) -> Result<()> {
    let password = if password_stdin {
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        input.trim_end_matches(['\r', '\n']).to_owned()
    } else {
        rpassword::prompt_password("密码: ")?
    };
    if password.is_empty() {
        bail!("密码不能为空");
    }
    account::login(username, &password).await?;
    println!("已登录: {username}");

    Ok(())
}

pub async fn whoami() -> Result<()> {
    match account::status().await? {
        Some(username) => println!("{username}"),
        None => println!("尚未登录"),
    }

    Ok(())
}

pub fn logout() -> Result<()> {
    account::logout()?;
    println!("已退出登录");

    Ok(())
}
