/// Trait for sending emails.
pub trait MailProvider: Send + Sync {
    fn send(&self, to: &str, subject: &str, body: &str) -> anyhow::Result<()>;
}

/// Development mail provider that logs emails to stdout.
pub struct LogMailProvider;

impl MailProvider for LogMailProvider {
    fn send(&self, to: &str, subject: &str, body: &str) -> anyhow::Result<()> {
        println!("[MAIL] To: {to}\n[MAIL] Subject: {subject}\n[MAIL] Body:\n{body}");
        Ok(())
    }
}
