// CO-38: Space Invaders — classic shoot-em-up
// Pure HTML5 canvas. Posts score to /api/v1/games/invaders/result on game over.

(function () {
    'use strict';

    const W = 480, H = 480;
    const ROWS = 4, COLS = 10;
    const ALIEN_W = 32, ALIEN_H = 24, ALIEN_GAP = 8;
    const PLAYER_W = 40, PLAYER_H = 10;
    const BULLET_W = 3, BULLET_H = 10;

    let canvas, ctx, scoreEl, livesEl, bestEl;
    let player, bullets, aliens, alienDir, alienDropping;
    let score, lives, best, gameOver, won, animFrame;

    function init(container) {
        container.innerHTML = `
            <div class="inv-wrapper">
                <div class="inv-info">
                    <div class="inv-stat"><span>Pontuação</span><strong id="inv-score">0</strong></div>
                    <div class="inv-stat"><span>Vidas</span><strong id="inv-lives">3</strong></div>
                    <div class="inv-stat"><span>Melhor</span><strong id="inv-best">0</strong></div>
                </div>
                <canvas id="inv-canvas" width="${W}" height="${H}"></canvas>
            </div>
            <style>
                .inv-wrapper { display:flex; gap:16px; align-items:flex-start; justify-content:center; }
                .inv-info { display:flex; flex-direction:column; gap:12px; padding:12px; background:rgba(255,255,255,.05); border-radius:8px; min-width:80px; }
                .inv-stat { display:flex; flex-direction:column; gap:2px; font-size:12px; }
                .inv-stat strong { font-size:20px; font-family:'Newsreader', serif; }
                #inv-canvas { border:1px solid rgba(255,255,255,.15); border-radius:4px; display:block; }
            </style>
        `;

        canvas = document.getElementById('inv-canvas');
        ctx = canvas.getContext('2d');
        scoreEl = document.getElementById('inv-score');
        livesEl = document.getElementById('inv-lives');
        bestEl = document.getElementById('inv-best');

        best = parseInt(localStorage.getItem('co_invaders_best') || '0', 10);
        bestEl.textContent = best;

        resetGame();
        bindKeys();
        bindTouch();
        loop();
    }

    function resetGame() {
        player = { x: W / 2 - PLAYER_W / 2, y: H - 30, vx: 0 };
        bullets = [];
        alienDir = 1;
        alienDropping = false;
        score = 0;
        lives = 3;
        gameOver = false;
        won = false;
        scoreEl && (scoreEl.textContent = '0');
        livesEl && (livesEl.textContent = '3');
        spawnAliens();
    }

    function spawnAliens() {
        aliens = [];
        const startX = 30;
        const startY = 50;
        for (let r = 0; r < ROWS; r++) {
            for (let c = 0; c < COLS; c++) {
                aliens.push({
                    x: startX + c * (ALIEN_W + ALIEN_GAP),
                    y: startY + r * (ALIEN_H + ALIEN_GAP),
                    alive: true,
                    row: r,
                });
            }
        }
    }

    let lastAlienMove = 0;
    let lastShoot = 0;

    function loop(ts = 0) {
        animFrame = requestAnimationFrame(loop);
        if (gameOver || won) { draw(); return; }

        // Player movement
        player.x += player.vx;
        player.x = Math.max(0, Math.min(W - PLAYER_W, player.x));

        // Move bullets
        for (let i = bullets.length - 1; i >= 0; i--) {
            bullets[i].y += bullets[i].vy;
            if (bullets[i].y < -BULLET_H || bullets[i].y > H + BULLET_H) {
                bullets.splice(i, 1);
            }
        }

        // Move aliens
        if (ts - lastAlienMove > alienSpeed()) {
            lastAlienMove = ts;
            let hitWall = false;
            const alive = aliens.filter(a => a.alive);
            alive.forEach(a => {
                a.x += alienDir * 8;
                if (a.x <= 0 || a.x + ALIEN_W >= W) hitWall = true;
            });
            if (hitWall) {
                alienDir *= -1;
                alive.forEach(a => { a.y += 16; });
                if (alive.some(a => a.y + ALIEN_H >= player.y)) {
                    endGame(false);
                    return;
                }
            }
        }

        // Alien shooting
        if (ts - lastShoot > 1200) {
            lastShoot = ts;
            const alive = aliens.filter(a => a.alive);
            if (alive.length > 0) {
                const shooter = alive[Math.floor(Math.random() * alive.length)];
                bullets.push({ x: shooter.x + ALIEN_W / 2, y: shooter.y + ALIEN_H, vy: 4, alien: true });
            }
        }

        // Check bullet collisions
        for (let i = bullets.length - 1; i >= 0; i--) {
            const b = bullets[i];
            if (!b.alien) {
                // Player bullet vs aliens
                for (let j = aliens.length - 1; j >= 0; j--) {
                    const a = aliens[j];
                    if (!a.alive) continue;
                    if (b.x >= a.x && b.x <= a.x + ALIEN_W && b.y >= a.y && b.y <= a.y + ALIEN_H) {
                        a.alive = false;
                        bullets.splice(i, 1);
                        score += (ROWS - a.row) * 10;
                        scoreEl && (scoreEl.textContent = score);
                        break;
                    }
                }
            } else {
                // Alien bullet vs player
                if (b.x >= player.x && b.x <= player.x + PLAYER_W &&
                    b.y >= player.y && b.y <= player.y + PLAYER_H) {
                    bullets.splice(i, 1);
                    lives--;
                    livesEl && (livesEl.textContent = lives);
                    if (lives <= 0) { endGame(false); return; }
                }
            }
        }

        if (aliens.every(a => !a.alive)) {
            endGame(true);
        }

        draw();
    }

    function alienSpeed() {
        const alive = aliens.filter(a => a.alive).length;
        return Math.max(80, 300 - (ROWS * COLS - alive) * 6);
    }

    function draw() {
        if (!ctx) return;
        ctx.clearRect(0, 0, W, H);
        ctx.fillStyle = '#000';
        ctx.fillRect(0, 0, W, H);

        if (!gameOver && !won) {
            // Player
            ctx.fillStyle = 'var(--accent4, #34d399)';
            ctx.fillRect(player.x, player.y, PLAYER_W, PLAYER_H);

            // Aliens
            aliens.forEach(a => {
                if (!a.alive) return;
                const hue = a.row < 2 ? 'var(--accent, #e0505f)' : 'var(--accent3, #60a5fa)';
                ctx.fillStyle = hue;
                // Simple alien shape
                ctx.fillRect(a.x + 6, a.y, ALIEN_W - 12, ALIEN_H - 8);
                ctx.fillRect(a.x, a.y + 6, ALIEN_W, ALIEN_H - 14);
                ctx.fillRect(a.x + 2, a.y + ALIEN_H - 8, 8, 8);
                ctx.fillRect(a.x + ALIEN_W - 10, a.y + ALIEN_H - 8, 8, 8);
            });

            // Bullets
            bullets.forEach(b => {
                ctx.fillStyle = b.alien ? 'var(--accent2, #E9C349)' : '#fff';
                ctx.fillRect(b.x - BULLET_W / 2, b.y, BULLET_W, BULLET_H);
            });
        }

        if (gameOver || won) {
            ctx.fillStyle = 'rgba(0,0,0,0.75)';
            ctx.fillRect(0, 0, W, H);
            ctx.fillStyle = '#fff';
            ctx.font = 'bold 22px Manrope, sans-serif';
            ctx.textAlign = 'center';
            ctx.fillText(won ? 'VITÓRIA!' : 'GAME OVER', W / 2, H / 2 - 20);
            ctx.font = '14px Manrope, sans-serif';
            ctx.fillText(`Pontuação: ${score}`, W / 2, H / 2 + 10);
            ctx.fillText('Toque para jogar de novo', W / 2, H / 2 + 35);
        }
    }

    function endGame(victory) {
        gameOver = !victory;
        won = victory;
        if (score > best) {
            best = score;
            localStorage.setItem('co_invaders_best', best);
            bestEl && (bestEl.textContent = best);
        }
        fetch('/api/v1/games/invaders/result', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            credentials: 'include',
            body: JSON.stringify({ score }),
        }).catch(() => {});
    }

    function shoot() {
        if (gameOver || won) return;
        if (bullets.filter(b => !b.alien).length >= 3) return;
        bullets.push({ x: player.x + PLAYER_W / 2, y: player.y, vy: -8, alien: false });
    }

    function bindKeys() {
        document.addEventListener('keydown', e => {
            if (gameOver || won) { if (e.key === 'Enter' || e.key === ' ') resetGame(); return; }
            if (e.key === 'ArrowLeft')  player.vx = -4;
            if (e.key === 'ArrowRight') player.vx = 4;
            if (e.key === ' ' || e.key === 'ArrowUp') shoot();
        });
        document.addEventListener('keyup', e => {
            if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') player.vx = 0;
        });
    }

    function bindTouch() {
        let startX;
        canvas.addEventListener('touchstart', e => {
            if (gameOver || won) { resetGame(); return; }
            startX = e.touches[0].clientX;
            shoot();
            e.preventDefault();
        }, { passive: false });
        canvas.addEventListener('touchmove', e => {
            const dx = e.touches[0].clientX - startX;
            player.vx = dx * 0.08;
            e.preventDefault();
        }, { passive: false });
        canvas.addEventListener('touchend', () => { player.vx = 0; }, { passive: false });
    }

    window.CoInvaders = { init };
})();
