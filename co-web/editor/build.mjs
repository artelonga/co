#!/usr/bin/env node
import * as esbuild from 'esbuild';
import { argv } from 'process';

const watch = argv.includes('--watch');
const outfile = '../static/shared/editor.bundle.js';

const ctx = await esbuild.context({
  entryPoints: ['src/editor.js'],
  bundle: true,
  minify: !watch,
  format: 'iife',
  globalName: 'CoEditor',
  outfile,
  treeShaking: true,
  logLevel: 'info',
});

if (watch) {
  await ctx.watch();
  console.log('Watching for changes…');
} else {
  await ctx.rebuild();
  await ctx.dispose();
}
