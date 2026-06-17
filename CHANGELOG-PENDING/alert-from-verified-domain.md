## alert-from-verified-domain — degradation alerts send from a Resend-verified domain

Changed the default `CO_ALERT_FROM` from `CO Alertas <alertas@artelonga.com.br>`
to `CO Alertas <alertas@seguranca.artelonga.com.br>`.

### Why
`artelonga.com.br` is not a Resend-verified sending domain, so disk-pressure /
degradation alert emails (CO-422) failed to send or were spam-foldered.
`seguranca.artelonga.com.br` is the verified domain already used for password and
notification mail (`senhas@`, `notificacoes@`). Overridable via the `CO_ALERT_FROM`
secret.
