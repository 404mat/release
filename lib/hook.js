// Native
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

// Utilities
import * as handleSpinner from './spinner.js';

export default async (flag, markdown, changes) => {
  let file = resolve(process.cwd(), 'release.js');

  if (!flag && !existsSync(file)) {
    return markdown;
  }

  if (flag) {
    file = resolve(process.cwd(), flag);

    if (!existsSync(file)) {
      handleSpinner.fail(`The specified ${'--hook'} file doesn't exist`);
    }
  }

  let hook;

  try {
    const hookModule = await import(pathToFileURL(file).href);
    hook = hookModule.default ?? hookModule;
  } catch (err) {
    handleSpinner.fail(err);
  }

  if (typeof hook !== 'function') {
    handleSpinner.fail(`The release hook file doesn't export a function`);
  }

  if (global.spinner) {
    global.spinner.succeed('Found a hook file');
  }

  let filtered;

  try {
    filtered = await hook(markdown, changes);
  } catch (err) {
    handleSpinner.fail(err);
  }

  return filtered;
};
