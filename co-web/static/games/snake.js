// CO-38: Snake — classic snake game
// Pure HTML5 canvas. Posts score to /api/v1/games/snake/result on game over.

(function () {
    'use strict';

    const GRID = 20;
    const CELL = 20;
    const W = GRID * CELL;
    const H = GRID * CELL;

    let canvas, ctx, scoreEl, bestEl;
    let snake, dir, nextDir, food, score, best;
    let timer, gameOver, started;

    function init(container) {
        container.innerHTML = `
            <div class="snake-wrapper">
                <div class="snake-info">
                    <div class="snake-stat"><span>Pontuação</span><strong id="snk-score">0</strong></div>
                    <div class="snake-stat"><span>Melhor</span><strong id="snk-best">0</strong></div>
                </div>
                <canvas id="snk-canvas" width="${W}" height="${H}"></canvas>
            </div>
            <style>
                .snake-wrapper { display:flex; gap:16px; align-items:flex-start; justify-content:center; }
                .snake-info { display:flex; flex-direction:column; gap:12px; padding:12px; background:rgba(255,255,255,.05); border-radius:8px; min-width:80px; }
                .snake-stat { display:flex; flex-direction:column; gap:2px; font-size:12px; }
                .snake-stat strong { font-size:20px; font-family:'Newsreader', serif; }
                #snk-canvas { border:1px solid rgba(255,255,255,.15); border-radius:4px; display:block; }
            </style>
        `;

        canvas = document.getElementById('snk-canvas');
        ctx = canvas.getContext('2d');
        scoreEl = document.getElementById('snk-score');
        bestEl = document.getElementById('snk-best');

        best = parseInt(localStorage.getItem('co_snake_best') || '0', 10);
        bestEl.textContent = best;

        resetGame();
        bindKeys();
        bindTouch();
    }

    function resetGame() {
        snake = [{ x: 10, y: 10 }, { x: 9, y: 10 }, { x: 8, y: 10 }];
        dir = { x: 1, y: 0 };
        nextDir = { x: 1, y: 0 };
        score = 0;
        gameOver = false;
        started = true;
        scoreEl && (scoreEl.textContent = '0');
        placeFood();
        clearInterval(timer);
        timer = setInterval(tick, 120);
    }

    function placeFood() {
        let pos;
        do {
            pos = { x: Math.floor(Math.random() * GRID), y: Math.floor(Math.random() * GRID) };
        } while (snake.some(s => s.x === pos.x && s.y === pos.y));
        food = pos;
    }

    function tick() {
        if (gameOver) return;
        dir = nextDir;
        const head = { x: snake[0].x + dir.x, y: snake[0].y + dir.y };
        if (head.x < 0 || head.x >= GRID || head.y < 0 || head.y >= GRID ||
            snake.some(s => s.x === head.x && s.y === head.y)) {
            endGame();
            return;
        }
        snake.unshift(head);
        if (head.x === food.x && head.y === food.y) {
            score += 10;
            scoreEl && (scoreEl.textContent = score);
            placeFood();
        } else {
            snake.pop();
        }
        draw();
    }

    function draw() {
        if (!ctx) return;
        ctx.clearRect(0, 0, W, H);
        ctx.fillStyle = 'rgba(0,0,0,0.6)';
        ctx.fillRect(0, 0, W, H);

        // Food
        ctx.fillStyle = 'var(--accent2, #E9C349)';
        ctx.beginPath();
        ctx.arc(food.x * CELL + CELL / 2, food.y * CELL + CELL / 2, CELL / 2 - 2, 0, Math.PI * 2);
        ctx.fill();

        // Snake
        snake.forEach((s, i) => {
            ctx.fillStyle = i === 0 ? 'var(--accent, #e0505f)' : 'var(--accent4, #34d399)';
            ctx.fillRect(s.x * CELL + 1, s.y * CELL + 1, CELL - 2, CELL - 2);
        });

        if (gameOver) {
            ctx.fillStyle = 'rgba(0,0,0,0.7)';
            ctx.fillRect(0, 0, W, H);
            ctx.fillStyle = '#fff';
            ctx.font = 'bold 20px Manrope, sans-serif';
            ctx.textAlign = 'center';
            ctx.fillText('GAME OVER', W / 2, H / 2 - 20);
            ctx.font = '14px Manrope, sans-serif';
            ctx.fillText(`Pontuação: ${score}`, W / 2, H / 2 + 10);
            ctx.fillText('Toque para jogar de novo', W / 2, H / 2 + 35);
        }
    }

    function endGame() {
        gameOver = true;
        clearInterval(timer);
        if (score > best) {
            best = score;
            localStorage.setItem('co_snake_best', best);
            bestEl && (bestEl.textContent = best);
        }
        const token = getCookie('session');
        if (token) {
            fetch('/api/v1/games/snake/result', {
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
            if (gameOver) { if (e.key === 'Enter' || e.key === ' ') resetGame(); return; }
            if (e.key === 'ArrowUp'    && dir.y !== 1)  nextDir = { x: 0, y: -1 };
            if (e.key === 'ArrowDown'  && dir.y !== -1) nextDir = { x: 0, y:  1 };
            if (e.key === 'ArrowLeft'  && dir.x !== 1)  nextDir = { x: -1, y: 0 };
            if (e.key === 'ArrowRight' && dir.x !== -1) nextDir = { x:  1, y: 0 };
        });
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
            const dx = e.changedTouches[0].clientX - startX;
            const dy = e.changedTouches[0].clientY - startY;
            if (Math.abs(dx) > Math.abs(dy)) {
                if (dx > 15 && dir.x !== -1) nextDir = { x: 1, y: 0 };
                else if (dx < -15 && dir.x !== 1) nextDir = { x: -1, y: 0 };
            } else {
                if (dy > 15 && dir.y !== -1) nextDir = { x: 0, y: 1 };
                else if (dy < -15 && dir.y !== 1) nextDir = { x: 0, y: -1 };
            }
            e.preventDefault();
        }, { passive: false });
    }

    window.CoSnake = { init };
})();
