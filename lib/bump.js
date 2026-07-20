// Native
import { exec, execFile } from 'node:child_process';
import path from 'node:path';
import { promisify } from 'node:util';

// Packages
import chalk from 'chalk';
import fs from 'fs-extra';
import semver from 'semver';

// Utilities
import { create as createSpinner, fail } from './spinner.js';
import getTags from './tags.js';

const increment = async (type, preSuffix) => {
  const pkgPath = path.join(process.cwd(), 'package.json');

  if (!fs.existsSync(pkgPath)) {
    throw new Error(`The "package.json" file doesn't exist`);
  }

  let pkgContent;

  try {
    pkgContent = await fs.readJSON(pkgPath);
  } catch {
    throw new Error(`Couldn't parse "package.json"`);
  }

  if (!pkgContent.version) {
    throw new Error(`No "version" field inside "package.json"`);
  }

  const { version: packageVersion } = pkgContent;
  const oldVersion = semver.valid(packageVersion);

  if (!oldVersion) {
    throw new Error(`The "package.json" version is not SemVer-compatible`);
  }

  const [latestTag] = await getTags();
  let tagPrefix = '';

  if (latestTag) {
    if (oldVersion !== latestTag.version) {
      throw new Error(
        `The "package.json" version (${packageVersion}) doesn't match the latest release tag (${latestTag.tag})`
      );
    }

    tagPrefix = latestTag.tag.slice(0, -latestTag.version.length);
  }

  const isPre = semver.prerelease(oldVersion);
  const shouldBePre = type === 'pre';

  if (!isPre && shouldBePre && !preSuffix) {
    preSuffix = 'canary';
  }

  let newVersion;

  if (shouldBePre && preSuffix) {
    newVersion = semver.inc(oldVersion, type, preSuffix);
  } else {
    newVersion = semver.inc(oldVersion, type);
  }

  pkgContent.version = newVersion;

  try {
    await fs.writeJSON(pkgPath, pkgContent, {
      spaces: 2,
    });
  } catch {
    throw new Error(`Couldn't write to "package.json"`);
  }

  const lockfilePath = path.join(process.cwd(), 'package-lock.json');

  if (!fs.existsSync(lockfilePath)) {
    return {
      version: newVersion,
      tag: `${tagPrefix}${newVersion}`,
    };
  }

  let lockfileContent;

  try {
    lockfileContent = await fs.readJSON(lockfilePath);
  } catch {
    throw new Error(`Couldn't parse "package-lock.json"`);
  }

  lockfileContent.version = newVersion;

  try {
    await fs.writeJSON(lockfilePath, lockfileContent, {
      spaces: 2,
    });
  } catch {
    throw new Error(`Couldn't write to "package-lock.json"`);
  }

  return {
    version: newVersion,
    tag: `${tagPrefix}${newVersion}`,
  };
};

const runGitCommand = async (command) => {
  try {
    const { stdout } = await promisify(exec)(command);
    return stdout;
  } catch (err) {
    if (err.message.includes('Not a git repository')) {
      throw new Error('Directory is not a Git repository');
    }

    throw err;
  }
};

const runGitArgs = async (args) => {
  try {
    await promisify(execFile)('git', args);
  } catch (err) {
    if (err.message.includes('Not a git repository')) {
      throw new Error('Directory is not a Git repository');
    }

    throw err;
  }
};

export const createAnnotatedTag = async (tag, changelog) => {
  createSpinner('Tagging commit');

  try {
    await runGitArgs(['tag', '-a', tag, '-m', changelog]);
  } catch (err) {
    fail(err.message);
  }

  global.spinner.text = `Tagged commit`;
  createSpinner('Pushing everything to remote');

  try {
    await runGitArgs(['push']);
    await runGitArgs(['push', '--tags']);
  } catch (err) {
    fail(err.message);
  }

  global.spinner.succeed(`Pushed everything to remote`);
  global.spinner = null;
};

export default async (type, preSuffix, tagOnly = false) => {
  createSpinner('Bumping version tag');
  let release;

  try {
    release = await increment(type, preSuffix);
  } catch (err) {
    fail(err.message);
  }

  const { tag } = release;

  global.spinner.text = `Bumped version tag to ${chalk.bold(tag)}`;
  createSpinner('Creating release commit');

  try {
    await runGitCommand(`git add -A && git commit -a -m "${tag}"`);
  } catch (err) {
    fail(err.message);
  }

  global.spinner.text = `Created release commit`;

  if (tagOnly) {
    let hash;

    try {
      hash = (await runGitCommand('git rev-parse HEAD')).trim();
    } catch (err) {
      fail(err.message);
    }

    global.spinner.succeed();
    global.spinner = null;

    return {
      ...release,
      hash,
      date: new Date(),
    };
  }

  createSpinner('Tagging commit');

  try {
    await runGitCommand(`git tag ${tag}`);
  } catch (err) {
    fail(err.message);
  }

  global.spinner.text = `Tagged commit`;
  createSpinner('Pushing everything to remote');

  try {
    await runGitCommand(`git push && git push --tags`);
  } catch (err) {
    fail(err.message);
  }

  global.spinner.succeed(`Pushed everything to remote`);
  global.spinner = null;
};
