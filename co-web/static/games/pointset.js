// CO-38: PointSet — point-and-click memory puzzle
// Find matching pairs of colored dots. Posts score to /api/v1/games/pointset/result.

(function () {
    'use strict';

    const GRID_SIZE = 4; // 4x4 = 16 cards = 8 pairs
    const CARD_SIZE = 64;
    const GAP = 8;
    const W = GRID_SIZE * (CARD_SIZE + GAP) - GAP;
    const H = GRID_SIZE * (CARD_SIZE + GAP) - GAP;

    const COLORS = [
        '#e0505f', '#E9C349', '#60a5fa', '#34d399',
        '#f97316', '#a78bfa', '#fb7185', '#22d3ee',
    ];

    let canvas, ctx, scoreEl, movesEl, bestEl;
    let cards, flipped, matched, moves, score, best, gameOver, animFrame;
    let lockBoard = false;

    function init(container) {
        container.innerHTML = `
            <div class="ps-wrapper">
                <div class="ps-info">
                    <div class="ps-stat"><span>Pontos</span><strong id="ps-score">0</strong></div>
                    <div class="ps-stat"><span>Movimentos</span><strong id="ps-moves">0</strong></div>
                    <div class="ps-stat"><span>Melhor</span><strong id="ps-best">0</strong></div>
                </div>
                <canvas id="ps-canvas" width="${W}" height="${H}"></canvas>
            </div>
            <style>
                .ps-wrapper { display:flex; gap:16px; align-items:flex-start; justify-content:center; }
                .ps-info { display:flex; flex-direction:column; gap:12px; padding:12px; background:rgba(255,255,255,.05); border-radius:8px; min-width:80px; }
                .ps-stat { display:flex; flex-direction:column; gap:2px; font-size:12px; }
                .ps-stat strong { font-size:20px; font-family:'Newsreader', serif; }
                #ps-canvas { border:1px solid rgba(255,255,255,.15); border-radius:4px; display:block; cursor:pointer; }
            </style>
        `;

        canvas = document.getElementById('ps-canvas');
        ctx = canvas.getContext('2d');
        scoreEl = document.getElementById('ps-score');
        movesEl = document.getElementById('ps-moves');
        bestEl = document.getElementById('ps-best');

        best = parseInt(localStorage.getItem('co_pointset_best') || '0', 10);
        bestEl.textContent = best;

        resetGame();
        canvas.addEventListener('click', handleClick);
        canvas.addEventListener('touchend', e => {
            const rect = canvas.getBoundingClientRect();
            const scaleX = canvas.width / rect.width;
            const scaleY = canvas.height / rect.height;
            const x = (e.changedTouches[0].clientX - rect.left) * scaleX;
            const y = (e.changedTouches[0].clientY - rect.top) * scaleY;
            handleClickAt(x, y);
            e.preventDefault();
        }, { passive: false });

        animFrame = requestAnimationFrame(loop);
    }

    function resetGame() {
        const pairs = [...COLORS, ...COLORS];
        // Fisher-Yates shuffle
        for (let i = pairs.length - 1; i > 0; i--) {
            const j = Math.floor(Math.random() * (i + 1));
            [pairs[i], pairs[j]] = [pairs[j], pairs[i]];
        }
        cards = pairs.map((color, i) => ({
            color,
            x: (i % GRID_SIZE) * (CARD_SIZE + GAP),
            y: Math.floor(i / GRID_SIZE) * (CARD_SIZE + GAP),
            revealed: false,
            matched: false,
        }));
        flipped = [];
        matched = 0;
        moves = 0;
        score = 0;
        gameOver = false;
        lockBoard = false;
        scoreEl && (scoreEl.textContent = '0');
        movesEl && (movesEl.textContent = '0');
    }

    function handleClick(e) {
        const rect = canvas.getBoundingClientRect();
        const scaleX = canvas.width / rect.width;
        const scaleY = canvas.height / rect.height;
        handleClickAt((e.clientX - rect.left) * scaleX, (e.clientY - rect.top) * scaleY);
    }

    function handleClickAt(mx, my) {
        if (lockBoard) return;
        if (gameOver) { resetGame(); return; }

        for (let i = 0; i < cards.length; i++) {
            const c = cards[i];
            if (c.matched || c.revealed) continue;
            if (mx >= c.x && mx <= c.x + CARD_SIZE && my >= c.y && my <= c.y + CARD_SIZE) {
                c.revealed = true;
                flipped.push(i);

                if (flipped.length === 2) {
                    moves++;
                    movesEl && (movesEl.textContent = moves);
                    const [a, b] = flipped;
                    if (cards[a].color === cards[b].color) {
                        cards[a].matched = true;
                        cards[b].matched = true;
                        matched += 2;
                        score += Math.max(0, 100 - moves * 2);
                        scoreEl && (scoreEl.textContent = score);
                        flipped = [];
                        if (matched === cards.length) endGame();
                    } else {
                        lockBoard = true;
                        setTimeout(() => {
                            cards[a].revealed = false;
                            cards[b].revealed = false;
                            flipped = [];
                            lockBoard = false;
                        }, 900);
                    }
                }
                break;
            }
        }
    }

    function loop() {
        draw();
        animFrame = requestAnimationFrame(loop);
    }

    function draw() {
        if (!ctx) return;
        ctx.clearRect(0, 0, W, H);
        ctx.fillStyle = 'rgba(0,0,0,0.4)';
        ctx.fillRect(0, 0, W, H);

        cards.forEach(c => {
            if (c.revealed || c.matched) {
                ctx.fillStyle = c.color;
                ctx.beginPath();
                ctx.roundRect(c.x + 4, c.y + 4, CARD_SIZE - 8, CARD_SIZE - 8, 8);
                ctx.fill();
                if (c.matched) {
                    ctx.strokeStyle = '#fff';
                    ctx.lineWidth = 2;
                    ctx.stroke();
                }
            } else {
                ctx.fillStyle = 'rgba(255,255,255,0.08)';
                ctx.beginPath();
                ctx.roundRect(c.x, c.y, CARD_SIZE, CARD_SIZE, 8);
                ctx.fill();
                ctx.strokeStyle = 'rgba(255,255,255,0.15)';
                ctx.lineWidth = 1;
                ctx.stroke();
                // Question mark
                ctx.fillStyle = 'rgba(255,255,255,0.3)';
                ctx.font = 'bold 24px Manrope, sans-serif';
                ctx.textAlign = 'center';
                ctx.textBaseline = 'middle';
                ctx.fillText('?', c.x + CARD_SIZE / 2, c.y + CARD_SIZE / 2);
            }
        });

        if (gameOver) {
            ctx.fillStyle = 'rgba(0,0,0,0.75)';
            ctx.fillRect(0, 0, W, H);
            ctx.fillStyle = '#fff';
            ctx.font = 'bold 20px Manrope, sans-serif';
            ctx.textAlign = 'center';
            ctx.textBaseline = 'alphabetic';
            ctx.fillText('PARABÉNS!', W / 2, H / 2 - 20);
            ctx.font = '14px Manrope, sans-serif';
            ctx.fillText(`Pontuação: ${score}`, W / 2, H / 2 + 10);
            ctx.fillText('Toque para jogar de novo', W / 2, H / 2 + 35);
        }
    }

    function endGame() {
        gameOver = true;
        if (score > best) {
            best = score;
            localStorage.setItem('co_pointset_best', best);
            bestEl && (bestEl.textContent = best);
        }
        fetch('/api/v1/games/pointset/result', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            credentials: 'include',
            body: JSON.stringify({ score }),
        }).catch(() => {});
    }

    window.CoPointSet = { init };
})();
