// CO-38: Tetris — classic falling blocks game
// Pure HTML5 canvas, no framework. Posts score to /api/v1/games/tetris/result on game over.

(function () {
    'use strict';

    const COLS = 10;
    const ROWS = 20;
    const BLOCK = 28;
    const COLORS = [
        null,
        'var(--accent, #e0505f)',       // I
        'var(--accent2, #E9C349)',       // O
        'var(--accent3, #60a5fa)',       // T
        'var(--accent4, #34d399)',       // S
        'var(--accent5, #f97316)',       // Z
        'var(--accent6, #a78bfa)',       // J
        'var(--accent7, #fb7185)',       // L
    ];

    const PIECES = [
        null,
        [[0,1,0,0],[0,1,0,0],[0,1,0,0],[0,1,0,0]], // I
        [[1,1],[1,1]],                               // O
        [[0,1,0],[1,1,1],[0,0,0]],                   // T
        [[0,1,1],[1,1,0],[0,0,0]],                   // S
        [[1,1,0],[0,1,1],[0,0,0]],                   // Z
        [[1,0,0],[1,1,1],[0,0,0]],                   // J
        [[0,0,1],[1,1,1],[0,0,0]],                   // L
    ];

    let canvas, ctx, scoreEl, levelEl, bestEl;
    let board, piece, pieceX, pieceY, pieceType;
    let score, level, lines, best;
    let dropInterval, animFrame;
    let gameOver = false;
    let started = false;

    function init(container) {
        container.innerHTML = `
            <div class="tetris-wrapper">
                <div class="tetris-info">
                    <div class="tetris-stat"><span>Pontuação</span><strong id="tet-score">0</strong></div>
                    <div class="tetris-stat"><span>Nível</span><strong id="tet-level">1</strong></div>
                    <div class="tetris-stat"><span>Melhor</span><strong id="tet-best">0</strong></div>
                </div>
                <canvas id="tet-canvas" width="${COLS * BLOCK}" height="${ROWS * BLOCK}"></canvas>
            </div>
            <style>
                .tetris-wrapper { display:flex; gap:16px; align-items:flex-start; justify-content:center; }
                .tetris-info { display:flex; flex-direction:column; gap:12px; padding:12px; background:rgba(255,255,255,.05); border-radius:8px; min-width:80px; }
                .tetris-stat { display:flex; flex-direction:column; gap:2px; font-size:12px; }
                .tetris-stat strong { font-size:20px; font-family:'Newsreader', serif; }
                #tet-canvas { border:1px solid rgba(255,255,255,.15); border-radius:4px; display:block; }
            </style>
        `;

        canvas = document.getElementById('tet-canvas');
        ctx = canvas.getContext('2d');
        scoreEl = document.getElementById('tet-score');
        levelEl = document.getElementById('tet-level');
        bestEl = document.getElementById('tet-best');

        best = parseInt(localStorage.getItem('co_tetris_best') || '0', 10);
        bestEl.textContent = best;

        resetGame();
        bindKeys();
        bindTouch();
        loop();
    }

    function resetGame() {
        board = Array.from({ length: ROWS }, () => new Array(COLS).fill(0));
        score = 0;
        level = 1;
        lines = 0;
        gameOver = false;
        started = true;
        scoreEl && (scoreEl.textContent = '0');
        levelEl && (levelEl.textContent = '1');
        clearInterval(dropInterval);
        dropInterval = setInterval(drop, dropSpeed());
        spawnPiece();
    }

    function dropSpeed() {
        return Math.max(100, 500 - (level - 1) * 40);
    }

    function spawnPiece() {
        pieceType = Math.floor(Math.random() * 7) + 1;
        piece = PIECES[pieceType];
        pieceX = Math.floor((COLS - piece[0].length) / 2);
        pieceY = 0;
        if (collides()) endGame();
    }

    function collides(ox = 0, oy = 0, p = piece) {
        for (let r = 0; r < p.length; r++) {
            for (let c = 0; c < p[r].length; c++) {
                if (!p[r][c]) continue;
                const nx = pieceX + c + ox;
                const ny = pieceY + r + oy;
                if (nx < 0 || nx >= COLS || ny >= ROWS) return true;
                if (ny >= 0 && board[ny][nx]) return true;
            }
        }
        return false;
    }

    function drop() {
        if (gameOver) return;
        if (!collides(0, 1)) {
            pieceY++;
        } else {
            lock();
            clearLines();
            spawnPiece();
        }
        draw();
    }

    function lock() {
        for (let r = 0; r < piece.length; r++) {
            for (let c = 0; c < piece[r].length; c++) {
                if (!piece[r][c]) continue;
                const nx = pieceX + c;
                const ny = pieceY + r;
                if (ny >= 0) board[ny][nx] = pieceType;
            }
        }
    }

    function clearLines() {
        let cleared = 0;
        for (let r = ROWS - 1; r >= 0; r--) {
            if (board[r].every(v => v !== 0)) {
                board.splice(r, 1);
                board.unshift(new Array(COLS).fill(0));
                cleared++;
                r++;
            }
        }
        if (cleared > 0) {
            const pts = [0, 100, 300, 500, 800][cleared] * level;
            score += pts;
            lines += cleared;
            level = Math.floor(lines / 10) + 1;
            scoreEl && (scoreEl.textContent = score);
            levelEl && (levelEl.textContent = level);
            clearInterval(dropInterval);
            dropInterval = setInterval(drop, dropSpeed());
        }
    }

    function rotate(p) {
        return p[0].map((_, i) => p.map(r => r[i]).reverse());
    }

    function draw() {
        if (!ctx) return;
        ctx.clearRect(0, 0, canvas.width, canvas.height);
        ctx.fillStyle = 'rgba(0,0,0,0.6)';
        ctx.fillRect(0, 0, canvas.width, canvas.height);

        // Draw board
        for (let r = 0; r < ROWS; r++) {
            for (let c = 0; c < COLS; c++) {
                if (board[r][c]) {
                    drawBlock(c, r, COLORS[board[r][c]]);
                }
            }
        }

        // Draw piece
        if (piece) {
            for (let r = 0; r < piece.length; r++) {
                for (let c = 0; c < piece[r].length; c++) {
                    if (piece[r][c]) {
                        drawBlock(pieceX + c, pieceY + r, COLORS[pieceType]);
                    }
                }
            }
        }

        if (gameOver) {
            ctx.fillStyle = 'rgba(0,0,0,0.7)';
            ctx.fillRect(0, 0, canvas.width, canvas.height);
            ctx.fillStyle = '#fff';
            ctx.font = 'bold 20px Manrope, sans-serif';
            ctx.textAlign = 'center';
            ctx.fillText('GAME OVER', canvas.width / 2, canvas.height / 2 - 20);
            ctx.font = '14px Manrope, sans-serif';
            ctx.fillText(`Pontuação: ${score}`, canvas.width / 2, canvas.height / 2 + 10);
            ctx.fillText('Toque para jogar de novo', canvas.width / 2, canvas.height / 2 + 35);
        }
    }

    function drawBlock(x, y, color) {
        ctx.fillStyle = color;
        ctx.fillRect(x * BLOCK + 1, y * BLOCK + 1, BLOCK - 2, BLOCK - 2);
        ctx.fillStyle = 'rgba(255,255,255,0.15)';
        ctx.fillRect(x * BLOCK + 1, y * BLOCK + 1, BLOCK - 2, 4);
    }

    function loop() {
        draw();
        animFrame = requestAnimationFrame(loop);
    }

    function endGame() {
        gameOver = true;
        clearInterval(dropInterval);
        if (score > best) {
            best = score;
            localStorage.setItem('co_tetris_best', best);
            bestEl && (bestEl.textContent = best);
        }
        // Post score to server
        const token = getCookie('session');
        if (token) {
            fetch('/api/v1/games/tetris/result', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                credentials: 'include',
                body: JSON.stringify({ score }),
            }).catch(() => {});
        }
        draw();
    }

    function getCookie(name) {
        const m = document.cookie.match(new RegExp('(?:^|;)\\s*' + name + '=([^;]*)'));
        return m ? m[1] : null;
    }

    function bindKeys() {
        document.addEventListener('keydown', e => {
            if (gameOver) {
                if (e.key === 'Enter' || e.key === ' ') resetGame();
                return;
            }
            switch (e.key) {
                case 'ArrowLeft':  if (!collides(-1)) pieceX--; break;
                case 'ArrowRight': if (!collides(1))  pieceX++; break;
                case 'ArrowDown':  drop(); break;
                case 'ArrowUp': {
                    const r = rotate(piece);
                    if (!collides(0, 0, r)) piece = r;
                    break;
                }
                case ' ': hardDrop(); break;
            }
            draw();
        });
    }

    function hardDrop() {
        while (!collides(0, 1)) pieceY++;
        drop();
    }

    function bindTouch() {
        let startX, startY;
        canvas.addEventListener('touchstart', e => {
            if (gameOver) { resetGame(); return; }
            startX = e.touches[0].clientX;
            startY = e.touches[0].clientY;
            e.preventDefault();
        }, { passive: false });
        canvas.addEventListener('touchend', e => {
            if (!startX) return;
            const dx = e.changedTouches[0].clientX - startX;
            const dy = e.changedTouches[0].clientY - startY;
            if (Math.abs(dx) > Math.abs(dy)) {
                if (dx > 20 && !collides(1)) pieceX++;
                else if (dx < -20 && !collides(-1)) pieceX--;
            } else {
                if (dy > 20) drop();
                else if (dy < -20) { const r = rotate(piece); if (!collides(0, 0, r)) piece = r; }
            }
            draw();
            e.preventDefault();
        }, { passive: false });
    }

    // Export init for the Yggdrasil hub
    window.CoTetris = { init };
})();
