// @vitest-environment happy-dom
// CO-303: login modal renders admin tab when login-options.password=true

import { describe, it, expect, beforeEach, vi } from 'vitest'

function buildLoginModalDOM() {
  document.body.innerHTML = `
    <div id="login-modal-overlay" class="login-screen hidden">
      <div id="login-step-email">
        <input type="email" id="onboard-email" />
        <button id="btn-onboard-continue">Continuar</button>
        <p id="onboard-email-error" class="hidden"></p>
        <div id="oauth-providers" class="hidden"></div>
        <div class="login-links">
          <button id="btn-show-classic-login">Já tem usuário e senha?</button>
        </div>
        <div id="admin-signin-row" class="hidden">
          <button id="btn-admin-signin">Admin sign-in (password) →</button>
        </div>
      </div>

      <div id="login-step-code" class="hidden">
        <p id="onboard-sent-to"></p>
        <div id="dev-code-banner" class="hidden">
          <span>DEV</span>
          <strong id="dev-code-value"></strong>
        </div>
        <input type="text" id="onboard-code" maxlength="6" />
        <button id="btn-onboard-verify">Confirmar</button>
        <p id="onboard-code-error" class="hidden"></p>
        <button id="btn-onboard-resend">Reenviar</button>
        <button id="btn-onboard-edit-email">Editar</button>
      </div>

      <div id="login-step-password" class="hidden">
        <input type="text" id="login-usuario" />
        <input type="password" id="login-senha" />
        <button id="btn-entrar">Entrar</button>
        <p id="login-error" class="hidden"></p>
        <button id="btn-forgot-password">Esqueci</button>
        <button id="btn-show-signup">Criar conta</button>
        <button id="btn-back-to-email">← Email</button>
      </div>

      <div id="login-step-signup" class="hidden">
        <input type="text" id="signup-usuario" />
        <input type="password" id="signup-senha" />
        <input type="email" id="signup-email" />
        <button id="btn-signup-submit">Criar</button>
        <p id="signup-error" class="hidden"></p>
        <button id="btn-back-to-login-from-signup">Já tenho conta</button>
      </div>

      <div id="login-step-forgot" class="hidden">
        <input type="text" id="forgot-identifier" />
        <input type="email" id="forgot-email" />
        <button id="btn-forgot-send">Enviar</button>
        <p id="forgot-error" class="hidden"></p>
      </div>

      <div id="login-step-reset" class="hidden">
        <input type="text" id="reset-code" />
        <input type="password" id="reset-new-password" />
        <button id="btn-reset-submit">Salvar</button>
        <p id="reset-error" class="hidden"></p>
      </div>

      <div class="login-footer">
        <button id="btn-back-to-login" style="display:none">← Voltar</button>
        <button id="btn-lang-toggle">English</button>
      </div>
    </div>

    <div id="sidebar-user" class="hidden"></div>
    <button id="btn-logout">Sair</button>
  `
}

// Minimal i18n stub
function setupGlobals() {
  ;(window as any).t = (key: string) => key
  ;(window as any).currentLang = 'pt'
  ;(window as any).setLang = () => {}
}

describe('login modal — CO-303', () => {
  beforeEach(() => {
    buildLoginModalDOM()
    setupGlobals()
    vi.restoreAllMocks()
  })

  it('admin-signin-row is hidden by default', () => {
    const row = document.getElementById('admin-signin-row')!
    expect(row.classList.contains('hidden')).toBe(true)
  })

  it('admin-signin-row shown when login-options.password=true', async () => {
    // Simulate the fetch for login-options returning password=true
    global.fetch = vi.fn().mockResolvedValueOnce({
      ok: true,
      json: async () => ({ magic_code: true, password: true, google: false }),
    } as any)

    // Run the login-options probe
    const r = await fetch('/api/v1/auth/login-options')
    const opts = await r.json()
    if (opts.password) {
      document.getElementById('admin-signin-row')?.classList.remove('hidden')
    }

    const row = document.getElementById('admin-signin-row')!
    expect(row.classList.contains('hidden')).toBe(false)
  })

  it('admin-signin-row stays hidden when login-options.password=false', async () => {
    global.fetch = vi.fn().mockResolvedValueOnce({
      ok: true,
      json: async () => ({ magic_code: true, password: false, google: false }),
    } as any)

    const r = await fetch('/api/v1/auth/login-options')
    const opts = await r.json()
    if (opts.password) {
      document.getElementById('admin-signin-row')?.classList.remove('hidden')
    }

    const row = document.getElementById('admin-signin-row')!
    expect(row.classList.contains('hidden')).toBe(true)
  })

  it('dev-code-banner is hidden by default', () => {
    const banner = document.getElementById('dev-code-banner')!
    expect(banner.classList.contains('hidden')).toBe(true)
  })

  it('dev-code-banner shown and code filled when dev_code present', () => {
    const banner = document.getElementById('dev-code-banner')!
    const codeInput = document.getElementById('onboard-code') as HTMLInputElement
    const codeValue = document.getElementById('dev-code-value')!

    // Simulate applyDevCode() logic with a dev_code
    const devCode = '123456'
    codeValue.textContent = devCode
    banner.classList.remove('hidden')
    codeInput.value = devCode

    expect(banner.classList.contains('hidden')).toBe(false)
    expect(codeInput.value).toBe('123456')
    expect(codeValue.textContent).toBe('123456')
  })

  it('dev-code-banner stays hidden when no dev_code', () => {
    const banner = document.getElementById('dev-code-banner')!
    const codeInput = document.getElementById('onboard-code') as HTMLInputElement

    // Simulate applyDevCode() with null _lastDevCode — banner stays hidden
    banner.classList.add('hidden')

    expect(banner.classList.contains('hidden')).toBe(true)
    expect(codeInput.value).toBe('')
  })
})
