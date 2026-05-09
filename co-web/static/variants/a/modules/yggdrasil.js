// ===== Yggdrasil minigames hub =====
import { state } from './state.js';
import { apiFetch } from './api.js';
import { esc, formatTimeAgo, capitalize } from './helpers.js';
import { YGGDRASIL_GAMES, GAME_MODULES, GAME_GLOBALS } from './constants.js';

let _hideLoading = () => {};
let _hideLoginModal = () => {};
let _renderUserBadge = () => {};

export function injectYggdrasilCallbacks(callbacks) {
    _hideLoading = callbacks.hideLoading;
    _hideLoginModal = callbacks.hideLoginModal;
    _renderUserBadge = callbacks.renderUserBadge;
}

function readGameFromUrl() {
    const m = window.location.pathname.match(/^\/yggdrasil\/([a-z0-9-]+)$/);
    return m ? m[1] : null;
}

export async function bootYggdrasil() {
    state.isYggdrasil = true;
    document.documentElement.setAttribute('data-palette', 'relic');

    const me = await apiFetch('/api/v1/auth/me', {}, true);
    if (me) {
        _hideLoginModal();
        _renderUserBadge(me);
    }

    const gameId = state.gameView || readGameFromUrl();
    if (gameId) {
        state.gameView = gameId;
        if (me) {
            await renderYggdrasilGame(gameId, me);
        } else {
            renderYggdrasilLoginWall(gameId);
        }
    } else {
        if (me) {
            await renderYggdrasilHub(me);
        } else {
            renderYggdrasilLoginWall(null);
        }
    }
    _hideLoading();
}

export function renderYggdrasilLoginWall(gameId) {
    const content = document.getElementById('content');
    if (!content) return;
    document.getElementById('view-tabs') && (document.getElementById('view-tabs').style.display = 'none');
    document.getElementById('btn-new-task') && (document.getElementById('btn-new-task').style.display = 'none');
    const title = gameId ? `Faça login para jogar ${gameId.charAt(0).toUpperCase() + gameId.slice(1)}` : 'Yggdrasil — Hub de Jogos';
    content.innerHTML = `
        <div class="ygg-login-wall">
            <div class="ygg-login-card">
                <div class="ygg-login-icon material-symbols-outlined">sports_esports</div>
                <h2 class="ygg-login-title">${esc(title)}</h2>
                <p class="ygg-login-subtitle">Crie uma conta gratuita para acessar o hub de minijogos,<br>perfis de jogadores e rankings globais.</p>
                <div class="ygg-login-actions">
                    <button class="btn btn-primary ygg-btn-login" id="ygg-btn-login">Entrar</button>
                    <button class="btn btn-ghost ygg-btn-register" id="ygg-btn-register">Criar conta</button>
                </div>
            </div>
        </div>
        <style>
            .ygg-login-wall { display:flex; align-items:center; justify-content:center; min-height:60vh; padding:32px; }
            .ygg-login-card { text-align:center; padding:48px 40px; background:rgba(255,255,255,.04); border-radius:16px; border:1px solid rgba(255,255,255,.1); max-width:400px; width:100%; backdrop-filter:blur(8px); }
            .ygg-login-icon { font-size:56px; color:var(--accent, #e0505f); margin-bottom:16px; }
            .ygg-login-title { font-family:'Newsreader', serif; font-size:24px; margin:0 0 12px; }
            .ygg-login-subtitle { color:rgba(255,255,255,.6); font-size:14px; line-height:1.6; margin:0 0 28px; }
            .ygg-login-actions { display:flex; gap:12px; justify-content:center; flex-wrap:wrap; }
            .ygg-btn-login, .ygg-btn-register { min-width:120px; }
        </style>
    `;
    document.getElementById('ygg-btn-login') && document.getElementById('ygg-btn-login').addEventListener('click', () => {
        document.getElementById('btn-header-entrar') && document.getElementById('btn-header-entrar').click();
    });
    document.getElementById('ygg-btn-register') && document.getElementById('ygg-btn-register').addEventListener('click', () => {
        document.getElementById('btn-header-entrar') && document.getElementById('btn-header-entrar').click();
    });
}

export async function renderYggdrasilHub(me) {
    const content = document.getElementById('content');
    if (!content) return;
    document.getElementById('view-tabs') && (document.getElementById('view-tabs').style.display = 'none');
    document.getElementById('btn-new-task') && (document.getElementById('btn-new-task').style.display = 'none');
    const projectName = document.getElementById('project-name');
    if (projectName) projectName.textContent = 'Yggdrasil';

    const [globalBoard, recentAct, myProfile] = await Promise.all([
        apiFetch('/api/v1/games/leaderboard/global?limit=10', {}, true).catch(() => []),
        apiFetch('/api/v1/games/recent?limit=20', {}, true).catch(() => []),
        apiFetch('/api/v1/profile', {}, true).catch(() => null),
    ]);

    const gameStats = {};
    for (const g of YGGDRASIL_GAMES) {
        try {
            const s = await apiFetch(`/api/v1/games/${g.id}/stats`, {}, true);
            if (s) gameStats[g.id] = s;
        } catch (_) {}
    }

    content.innerHTML = `
        <div class="ygg-hub">
            <style>
                .ygg-hub { padding:24px; display:grid; grid-template-columns:1fr 320px; gap:24px; max-width:1100px; margin:0 auto; }
                @media(max-width:768px){ .ygg-hub { grid-template-columns:1fr; } }
                .ygg-section { margin-bottom:24px; }
                .ygg-section-title { font-family:'Newsreader', serif; font-size:18px; margin:0 0 14px; color:rgba(255,255,255,.8); border-bottom:1px solid rgba(255,255,255,.08); padding-bottom:8px; }
                .ygg-profile { background:rgba(255,255,255,.04); border-radius:12px; border:1px solid rgba(255,255,255,.08); padding:20px; backdrop-filter:blur(4px); }
                .ygg-profile-head { display:flex; align-items:center; gap:16px; margin-bottom:16px; }
                .ygg-avatar { width:52px; height:52px; border-radius:50%; background:var(--accent,#e0505f); display:flex; align-items:center; justify-content:center; font-size:22px; font-weight:700; color:#fff; font-family:'Newsreader',serif; }
                .ygg-profile-name { font-size:18px; font-family:'Newsreader',serif; }
                .ygg-profile-sub { font-size:12px; color:rgba(255,255,255,.5); margin-top:2px; }
                .ygg-hp-bar { height:6px; background:rgba(255,255,255,.1); border-radius:3px; overflow:hidden; margin-top:8px; }
                .ygg-hp-fill { height:100%; background:var(--accent,#e0505f); border-radius:3px; transition:width .4s; }
                .ygg-profile-stats { display:grid; grid-template-columns:1fr 1fr 1fr; gap:12px; margin-top:16px; }
                .ygg-pstat { text-align:center; font-size:11px; color:rgba(255,255,255,.5); }
                .ygg-pstat strong { display:block; font-size:18px; font-family:'Newsreader',serif; color:#fff; }
                .ygg-games { display:grid; grid-template-columns:repeat(auto-fill,minmax(160px,1fr)); gap:14px; }
                .ygg-game-card { background:rgba(255,255,255,.04); border:1px solid rgba(255,255,255,.08); border-radius:12px; padding:18px 14px; cursor:pointer; transition:transform .15s,border-color .15s,box-shadow .15s; text-align:center; }
                .ygg-game-card:hover { transform:scale(1.03); border-color:var(--accent,#e0505f); box-shadow:0 4px 20px rgba(224,80,95,.2); }
                .ygg-game-icon { font-size:32px; color:var(--accent,#e0505f); margin-bottom:8px; }
                .ygg-game-name { font-family:'Newsreader',serif; font-size:16px; margin-bottom:4px; }
                .ygg-game-desc { font-size:11px; color:rgba(255,255,255,.4); margin-bottom:10px; }
                .ygg-game-best { font-size:12px; color:rgba(255,255,255,.6); margin-bottom:10px; }
                .ygg-play-btn { display:inline-block; padding:6px 18px; background:var(--accent,#e0505f); color:#fff; border-radius:6px; font-size:12px; font-weight:600; letter-spacing:.5px; pointer-events:none; }
                .ygg-right { display:flex; flex-direction:column; gap:20px; }
                .ygg-lb { background:rgba(255,255,255,.04); border-radius:12px; border:1px solid rgba(255,255,255,.08); padding:16px; backdrop-filter:blur(4px); }
                .ygg-lb-row { display:flex; align-items:center; gap:10px; padding:6px 0; border-bottom:1px solid rgba(255,255,255,.05); font-size:13px; }
                .ygg-lb-row:last-child { border-bottom:none; }
                .ygg-lb-rank { width:20px; color:rgba(255,255,255,.4); font-size:11px; text-align:right; }
                .ygg-lb-rank.gold { color:#E9C349; font-weight:700; }
                .ygg-lb-name { flex:1; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
                .ygg-lb-score { color:var(--accent2,#E9C349); font-family:'Newsreader',serif; }
                .ygg-feed { background:rgba(255,255,255,.04); border-radius:12px; border:1px solid rgba(255,255,255,.08); padding:16px; backdrop-filter:blur(4px); }
                .ygg-feed-item { font-size:12px; color:rgba(255,255,255,.6); padding:5px 0; border-bottom:1px solid rgba(255,255,255,.04); }
                .ygg-feed-item:last-child { border-bottom:none; }
                .ygg-feed-item strong { color:#fff; }
                .ygg-empty { color:rgba(255,255,255,.3); font-size:12px; font-style:italic; }
            </style>
            <div class="ygg-left">
                <div class="ygg-section">
                    ${renderProfileCard(me, myProfile, gameStats)}
                </div>
                <div class="ygg-section">
                    <div class="ygg-section-title">Jogos</div>
                    <div class="ygg-games">
                        ${YGGDRASIL_GAMES.map(g => {
                            const st = gameStats[g.id];
                            const best = st ? st.high_score : 0;
                            return `<div class="ygg-game-card" data-game="${esc(g.id)}">
                                <div class="ygg-game-icon material-symbols-outlined">${esc(g.icon)}</div>
                                <div class="ygg-game-name">${esc(g.name)}</div>
                                <div class="ygg-game-desc">${esc(g.desc)}</div>
                                <div class="ygg-game-best">Melhor: ${best.toLocaleString()}</div>
                                <div class="ygg-play-btn">JOGAR</div>
                            </div>`;
                        }).join('')}
                    </div>
                </div>
            </div>
            <div class="ygg-right">
                <div class="ygg-lb">
                    <div class="ygg-section-title" style="margin-bottom:10px">Ranking Global</div>
                    ${globalBoard && globalBoard.length > 0
                        ? globalBoard.map(e => `
                            <div class="ygg-lb-row">
                                <span class="ygg-lb-rank${e.rank <= 3 ? ' gold' : ''}">${e.rank}</span>
                                <span class="ygg-lb-name">${esc(e.display_name || e.username || 'Jogador')}</span>
                                <span class="ygg-lb-score">${(e.total_score || 0).toLocaleString()}</span>
                            </div>`).join('')
                        : '<div class="ygg-empty">Nenhum jogador ainda.</div>'}
                </div>
                <div class="ygg-feed">
                    <div class="ygg-section-title" style="margin-bottom:10px">Atividade Recente</div>
                    ${recentAct && recentAct.length > 0
                        ? recentAct.slice(0, 10).map(e => {
                            const ago = formatTimeAgo(e.played_at);
                            return `<div class="ygg-feed-item">
                                <strong>${esc(e.display_name || e.username || 'Jogador')}</strong>
                                marcou ${(e.score || 0).toLocaleString()} em ${esc(capitalize(e.game_name))}
                                <span style="color:rgba(255,255,255,.3)"> · ${ago}</span>
                            </div>`;
                        }).join('')
                        : '<div class="ygg-empty">Nenhuma atividade ainda.</div>'}
                </div>
            </div>
        </div>
    `;

    content.querySelectorAll('.ygg-game-card').forEach(card => {
        card.addEventListener('click', () => {
            const gameId = card.dataset.game;
            navigateToGame(gameId, me);
        });
    });
}

function renderProfileCard(me, profile, gameStats) {
    const name = (me && me.display_name) || (me && me.email ? me.email.split('@')[0] : 'Aventureiro');
    const initial = name.charAt(0).toUpperCase();
    const totalScore = Object.values(gameStats).reduce((sum, s) => sum + (s.high_score || 0), 0);
    const gamesPlayed = Object.values(gameStats).reduce((sum, s) => sum + (s.games_played || 0), 0);
    const memberSince = (profile && profile.created_at)
        ? new Date(profile.created_at * 1000).getFullYear()
        : new Date().getFullYear();
    const level = Math.max(1, Math.floor(gamesPlayed / 5) + 1);
    const levelPct = Math.min(100, ((gamesPlayed % 5) / 5) * 100);

    return `<div class="ygg-profile">
        <div class="ygg-profile-head">
            <div class="ygg-avatar">${esc(initial)}</div>
            <div>
                <div class="ygg-profile-name">${esc(name)}</div>
                <div class="ygg-profile-sub">Nível ${level} · Aventureiro</div>
                <div class="ygg-hp-bar"><div class="ygg-hp-fill" style="width:${levelPct}%"></div></div>
            </div>
        </div>
        <div class="ygg-profile-stats">
            <div class="ygg-pstat"><strong>${totalScore.toLocaleString()}</strong>Pontuação Total</div>
            <div class="ygg-pstat"><strong>${gamesPlayed}</strong>Partidas</div>
            <div class="ygg-pstat"><strong>${memberSince}</strong>Membro desde</div>
        </div>
    </div>`;
}

export async function renderYggdrasilGame(gameId, me) {
    const content = document.getElementById('content');
    if (!content) return;
    document.getElementById('view-tabs') && (document.getElementById('view-tabs').style.display = 'none');
    document.getElementById('btn-new-task') && (document.getElementById('btn-new-task').style.display = 'none');
    const projectName = document.getElementById('project-name');
    if (projectName) projectName.textContent = 'Yggdrasil';

    const info = YGGDRASIL_GAMES.find(g => g.id === gameId);
    if (!info) { renderYggdrasilHub(me); return; }

    content.innerHTML = `
        <div class="ygg-game-view">
            <div class="ygg-game-header">
                <button class="ygg-back-btn" id="ygg-back-btn">
                    <span class="material-symbols-outlined">arrow_back</span>
                    <span>Yggdrasil</span>
                </button>
                <h2 class="ygg-game-title">${esc(info.name)}</h2>
            </div>
            <div class="ygg-game-area">
                <div id="ygg-game-canvas-area" class="ygg-game-canvas-area"></div>
                <div class="ygg-game-side">
                    <div id="ygg-game-lb" class="ygg-side-lb">
                        <div class="ygg-section-title">Top 10 — ${esc(info.name)}</div>
                        <div id="ygg-lb-list">Carregando...</div>
                    </div>
                </div>
            </div>
        </div>
        <style>
            .ygg-game-view { padding:16px 24px; }
            .ygg-game-header { display:flex; align-items:center; gap:16px; margin-bottom:20px; }
            .ygg-back-btn { display:flex; align-items:center; gap:6px; background:rgba(255,255,255,.08); border:1px solid rgba(255,255,255,.12); border-radius:8px; padding:6px 14px; cursor:pointer; color:inherit; font-size:14px; }
            .ygg-back-btn:hover { background:rgba(255,255,255,.12); }
            .ygg-game-title { font-family:'Newsreader',serif; font-size:22px; margin:0; }
            .ygg-game-area { display:flex; gap:24px; align-items:flex-start; flex-wrap:wrap; }
            .ygg-game-canvas-area { flex:1; min-width:280px; }
            .ygg-game-side { width:220px; flex-shrink:0; }
            .ygg-side-lb { background:rgba(255,255,255,.04); border-radius:12px; border:1px solid rgba(255,255,255,.08); padding:16px; }
            @media(max-width:600px){ .ygg-game-side { width:100%; } }
        </style>
    `;

    document.getElementById('ygg-back-btn').addEventListener('click', () => {
        state.gameView = null;
        window.history.pushState({}, '', '/yggdrasil');
        renderYggdrasilHub(me);
    });

    apiFetch(`/api/v1/games/${gameId}/leaderboard?limit=10`, {}, true).then(lb => {
        const el = document.getElementById('ygg-lb-list');
        if (!el) return;
        if (!lb || lb.length === 0) { el.innerHTML = '<div class="ygg-empty">Sem dados ainda.</div>'; return; }
        el.innerHTML = lb.map(e => `
            <div class="ygg-lb-row">
                <span class="ygg-lb-rank${e.rank <= 3 ? ' gold' : ''}">${e.rank}</span>
                <span class="ygg-lb-name">${esc(e.display_name || e.username || 'Jogador')}</span>
                <span class="ygg-lb-score">${(e.high_score || 0).toLocaleString()}</span>
            </div>`).join('');
    }).catch(() => {});

    const canvasArea = document.getElementById('ygg-game-canvas-area');
    await loadGameScript(GAME_MODULES[gameId]);
    const gameGlobal = window[GAME_GLOBALS[gameId]];
    if (gameGlobal && gameGlobal.init) {
        gameGlobal.init(canvasArea);
    }
}

function loadGameScript(src) {
    return new Promise((resolve, reject) => {
        if (document.querySelector(`script[src="${src}"]`)) { resolve(); return; }
        const s = document.createElement('script');
        s.src = src;
        s.onload = resolve;
        s.onerror = reject;
        document.head.appendChild(s);
    });
}

function navigateToGame(gameId, me) {
    state.gameView = gameId;
    window.history.pushState({}, '', `/yggdrasil/${gameId}`);
    if (me) renderYggdrasilGame(gameId, me);
    else renderYggdrasilLoginWall(gameId);
}
