/// Abstraction over email delivery.
pub trait MailProvider: Send + Sync {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String>;
}

/// Dev-mode provider: prints the email to stdout instead of sending it.
pub struct LogMailProvider;

impl MailProvider for LogMailProvider {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String> {
        println!("[MAIL] To: {to}");
        println!("[MAIL] Subject: {subject}");
        println!("[MAIL] Body: {body}");
        Ok(())
    }
}
