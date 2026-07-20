#!/usr/bin/env node

import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import inquirer from 'inquirer';

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

const skipPrompt =
  process.env.RELEASE_SKIP_GLOBAL_INSTALL_PROMPT === '1' ||
  process.env.npm_config_global === 'true' ||
  !process.stdin.isTTY ||
  !process.stdout.isTTY;

if (!skipPrompt) {
  const { installGlobally } = await inquirer.prompt([
    {
      type: 'confirm',
      name: 'installGlobally',
      message: 'Install Release globally with pnpm?',
      default: true,
    },
  ]);

  if (installGlobally) {
    const result = spawnSync('pnpm', ['add', '--global', packageRoot], {
      stdio: 'inherit',
      env: {
        ...process.env,
        RELEASE_SKIP_GLOBAL_INSTALL_PROMPT: '1',
      },
    });

    if (result.error) {
      console.error(`Failed to install Release globally: ${result.error.message}`);
      process.exitCode = 1;
    } else if (result.status !== 0) {
      process.exitCode = result.status;
    }
  }
}
