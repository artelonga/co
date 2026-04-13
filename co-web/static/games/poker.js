// CO-38: Video Poker — single-player 5-card draw poker
// Posts score to /api/v1/games/poker/result on game over.

(function () {
    'use strict';

    const SUITS = ['♠', '♥', '♦', '♣'];
    const RANKS = ['A','2','3','4','5','6','7','8','9','10','J','Q','K'];
    const HAND_NAMES = {
        'royal-flush':    { name: 'Royal Flush',    mult: 800 },
        'straight-flush': { name: 'Straight Flush', mult: 50 },
        'four-of-a-kind': { name: 'Quadra',         mult: 25 },
        'full-house':     { name: 'Full House',      mult: 9  },
        'flush':          { name: 'Flush',           mult: 6  },
        'straight':       { name: 'Reta',            mult: 4  },
        'three-of-a-kind':{ name: 'Trinca',          mult: 3  },
        'two-pair':       { name: 'Dois Pares',      mult: 2  },
        'jacks-or-better':{ name: 'Jacks ou Melhor', mult: 1  },
    };
    const BET = 1;

    let container, scoreEl, creditsEl, msgEl, handEl;
    let deck, hand, held, credits, totalScore, best, gameOver;
    let phase; // 'deal' | 'draw'

    function init(cont) {
        container = cont;
        container.innerHTML = `
            <div class="poker-wrapper">
                <div class="poker-top">
                    <div class="poker-stat"><span>Créditos</span><strong id="pk-credits">100</strong></div>
                    <div class="poker-stat"><span>Pontuação</span><strong id="pk-score">0</strong></div>
                </div>
                <div id="pk-hand" class="poker-hand"></div>
                <div id="pk-msg" class="poker-msg"></div>
                <div id="pk-result" class="poker-result"></div>
                <button id="pk-btn" class="poker-btn">DISTRIBUIR</button>
            </div>
            <style>
                .poker-wrapper { display:flex; flex-direction:column; align-items:center; gap:16px; max-width:520px; margin:0 auto; }
                .poker-top { display:flex; gap:32px; }
                .poker-stat { display:flex; flex-direction:column; align-items:center; font-size:12px; }
                .poker-stat strong { font-size:22px; font-family:'Newsreader', serif; }
                .poker-hand { display:flex; gap:8px; }
                .poker-card {
                    width:64px; height:96px; border-radius:8px; border:2px solid rgba(255,255,255,.2);
                    display:flex; flex-direction:column; align-items:center; justify-content:center;
                    font-size:22px; font-weight:700; font-family:'Newsreader', serif;
                    cursor:pointer; transition:transform .15s, border-color .15s;
                    background:rgba(255,255,255,.08);
                    user-select:none;
                }
                .poker-card.held { border-color:var(--accent2, #E9C349); transform:translateY(-8px); }
                .poker-card .suit { font-size:14px; margin-top:2px; }
                .poker-card.red { color:#f87171; }
                .poker-card.black { color:#e5e7eb; }
                .poker-msg { font-size:13px; color:rgba(255,255,255,.6); min-height:20px; }
                .poker-result { font-size:18px; font-family:'Newsreader', serif; min-height:24px; color:var(--accent2,#E9C349); }
                .poker-btn {
                    padding:10px 32px; background:var(--accent,#e0505f); color:#fff;
                    border:none; border-radius:8px; font-size:15px; font-weight:600;
                    cursor:pointer; letter-spacing:.5px;
                }
                .poker-btn:hover { opacity:.85; }
            </style>
        `;

        scoreEl = container.querySelector('#pk-score');
        creditsEl = container.querySelector('#pk-credits');
        msgEl = container.querySelector('#pk-msg');
        handEl = container.querySelector('#pk-hand');
        const btn = container.querySelector('#pk-btn');

        best = parseInt(localStorage.getItem('co_poker_best') || '0', 10);
        credits = 100;
        totalScore = 0;
        phase = 'deal';
        btn.addEventListener('click', onBtn);
        renderHand([]);
    }

    function onBtn() {
        if (phase === 'deal') dealHand();
        else drawCards();
    }

    function dealHand() {
        if (credits < BET) { endGame(); return; }
        credits -= BET;
        creditsEl.textContent = credits;
        deck = makeDeck();
        hand = [draw(), draw(), draw(), draw(), draw()];
        held = [false, false, false, false, false];
        phase = 'draw';
        container.querySelector('#pk-btn').textContent = 'TROCAR';
        container.querySelector('#pk-result').textContent = '';
        msgEl.textContent = 'Selecione as cartas para guardar';
        renderHand(hand);
    }

    function drawCards() {
        for (let i = 0; i < 5; i++) {
            if (!held[i]) hand[i] = draw();
        }
        held = [false, false, false, false, false];
        const result = evaluate(hand);
        const info = HAND_NAMES[result];
        const win = info ? info.mult * BET * 10 : 0;
        credits += win;
        totalScore += win;
        creditsEl.textContent = credits;
        scoreEl.textContent = totalScore;
        container.querySelector('#pk-result').textContent = info
            ? `${info.name} +${win}` : '';
        msgEl.textContent = win > 0 ? 'Você ganhou!' : 'Sem prêmio';
        phase = 'deal';
        container.querySelector('#pk-btn').textContent = 'DISTRIBUIR';
        renderHand(hand);
        if (totalScore > best) {
            best = totalScore;
            localStorage.setItem('co_poker_best', best);
        }
        if (credits <= 0) { endGame(); return; }
    }

    function endGame() {
        msgEl.textContent = 'Sem créditos — jogo encerrado';
        container.querySelector('#pk-btn').textContent = 'JOGAR DE NOVO';
        container.querySelector('#pk-btn').onclick = () => { credits = 100; totalScore = 0; phase = 'deal'; scoreEl.textContent = '0'; creditsEl.textContent = '100'; dealHand(); };
        fetch('/api/v1/games/poker/result', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            credentials: 'include',
            body: JSON.stringify({ score: totalScore }),
        }).catch(() => {});
    }

    function makeDeck() {
        const d = [];
        for (const s of SUITS) for (const r of RANKS) d.push({ rank: r, suit: s });
        for (let i = d.length - 1; i > 0; i--) {
            const j = Math.floor(Math.random() * (i + 1));
            [d[i], d[j]] = [d[j], d[i]];
        }
        return d;
    }

    function draw() { return deck.pop(); }

    function renderHand(cards) {
        handEl.innerHTML = cards.map((c, i) => {
            const isRed = c && (c.suit === '♥' || c.suit === '♦');
            const cls = `poker-card${isRed ? ' red' : ' black'}${held[i] ? ' held' : ''}`;
            return `<div class="${cls}" data-i="${i}">
                <span>${c ? c.rank : ''}</span>
                <span class="suit">${c ? c.suit : ''}</span>
            </div>`;
        }).join('');

        handEl.querySelectorAll('.poker-card').forEach(el => {
            el.addEventListener('click', () => {
                if (phase !== 'draw') return;
                const i = parseInt(el.dataset.i, 10);
                held[i] = !held[i];
                renderHand(hand);
            });
        });
    }

    function rankIndex(r) { return RANKS.indexOf(r); }

    function evaluate(hand) {
        const ranks = hand.map(c => rankIndex(c.rank)).sort((a, b) => a - b);
        const suits = hand.map(c => c.suit);
        const counts = {};
        ranks.forEach(r => { counts[r] = (counts[r] || 0) + 1; });
        const vals = Object.values(counts).sort((a, b) => b - a);
        const isFlush = suits.every(s => s === suits[0]);
        const isStraight = ranks[4] - ranks[0] === 4 && new Set(ranks).size === 5;
        const isRoyal = isFlush && ranks.join(',') === '8,9,10,11,12'; // 10,J,Q,K,A

        if (isRoyal) return 'royal-flush';
        if (isFlush && isStraight) return 'straight-flush';
        if (vals[0] === 4) return 'four-of-a-kind';
        if (vals[0] === 3 && vals[1] === 2) return 'full-house';
        if (isFlush) return 'flush';
        if (isStraight) return 'straight';
        if (vals[0] === 3) return 'three-of-a-kind';
        if (vals[0] === 2 && vals[1] === 2) return 'two-pair';
        if (vals[0] === 2) {
            // Jacks or better
            const pair = parseInt(Object.keys(counts).find(k => counts[k] === 2), 10);
            if (pair >= RANKS.indexOf('J')) return 'jacks-or-better';
        }
        return null;
    }

    window.CoPoker = { init };
})();
