use anyhow::Context;
use ownserp_core::check_for_update;

use crate::output::emit;

pub(crate) fn run(json: bool) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building the runtime")?;
    let status = runtime
        .block_on(check_for_update())
        .context("checking for a newer release")?;
    if json {
        emit(&status.to_json().to_string())?;
    } else {
        emit(&status.summary())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ownserp_core::UpdateStatus;

    #[test]
    fn a_json_answer_carries_every_field() {
        let status = UpdateStatus {
            current: "2.0.0".to_owned(),
            latest: Some("2.0.1".to_owned()),
            update_available: true,
            command: Some("brew upgrade ownserp"),
        };
        let value = status.to_json();
        assert_eq!(value["current"], "2.0.0");
        assert_eq!(value["latest"], "2.0.1");
        assert_eq!(value["updateAvailable"], true);
        assert_eq!(value["command"], "brew upgrade ownserp");
    }
}
