#!/usr/bin/env node

import { spawnSync } from 'node:child_process';

import inquirer from 'inquirer';

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
    const result = spawnSync('pnpm', ['link', '--global'], {
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
