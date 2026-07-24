#!/usr/bin/env node

// Native
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

// Packages
import args from 'args';
import chalk from 'chalk';
import inquirer from 'inquirer';
import open from 'open';
import semVer from 'semver';
import sleep from 'delay';

// Utilities
import applyHook from '../lib/hook.js';
import bumpVersion, { createAnnotatedTag } from '../lib/bump.js';
import connect from '../lib/connect.js';
import createChangelog from '../lib/changelog.js';
import * as definitions from '../lib/definitions.js';
import getChoices from '../lib/choices.js';
import getCommits from '../lib/commits.js';
import getTags from '../lib/tags.js';
import groupChanges from '../lib/group.js';
import pkg from '../package.json' with { type: 'json' };
import { branchSynced, getRepo } from '../lib/repo.js';
import { create as createSpinner, fail } from '../lib/spinner.js';

args
  .option('pre', 'Mark the release as prerelease')
  .option('overwrite', 'If the release already exists, replace it')
  .option('publish', 'Instead of creating a draft, publish the release')
  .option(['H', 'hook'], 'Specify a custom file to pipe releases through')
  .option(['t', 'previous-tag'], 'Specify previous release', '')
  .option(['S', 'summary'], 'Specify a short update summary', '')
  .option('tag-only', 'Create an annotated Git tag instead of a GitHub Release')
  .option('dry-run', 'Print the release without writing, tagging, pushing, or uploading')
  .option(['u', 'show-url'], 'Show the release URL instead of opening it in the browser')
  .option(
    ['s', 'skip-questions'],
    'Skip the questions and create a simple list without the headings'
  );

const flags = args.parse(process.argv);

// When running `release pre`, the release
// should automatically be marked as a pre-release
if (args.sub[0] === 'pre') {
  flags.pre = true;
}

let githubConnection;
let repoDetails;

const changeTypes = [
  {
    handle: 'major',
    name: 'Major Change',
    pluralName: 'Major Changes',
    description: 'incompatible API change',
  },
  {
    handle: 'minor',
    name: 'Minor Change',
    pluralName: 'Minor Changes',
    description: 'backwards-compatible functionality',
  },
  {
    handle: 'patch',
    name: 'Patch',
    pluralName: 'Patches',
    description: 'backwards-compatible bug fix',
  },
];

const checkForUpdate = async () => {
  const encoded = encodeURIComponent(pkg.name).replace(/^%40/, '@');
  const response = await fetch(`https://registry.npmjs.org/${encoded}`);

  if (!response.ok) {
    return null;
  }

  const data = await response.json();
  const latest = data['dist-tags'] && data['dist-tags'].latest;

  if (latest && semVer.gt(latest, pkg.version)) {
    return { latest };
  }

  return null;
};

const getReleaseURL = (release, edit = false) => {
  if (!release || !release.html_url) {
    return false;
  }

  const htmlURL = release.html_url;
  return edit ? htmlURL.replace('/tag/', '/edit/') : htmlURL;
};

const getHead = async () => {
  try {
    const { stdout } = await promisify(execFile)('git', ['rev-parse', 'HEAD']);
    return stdout.trim();
  } catch {
    fail('Directory is not a Git repository.');
  }
};

const getTagPrefix = (tag) => tag.slice(0, -semVer.valid(tag).length);

const getInferredReleaseType = (grouped) => {
  for (const type of changeTypes) {
    if (grouped[type.handle].length > 0) {
      return type.handle;
    }
  }

  return null;
};

const applyInferredDryRunTag = (tags, grouped) => {
  if (!flags.dryRun || !flags.tagOnly || args.sub.length > 0 || tags.length < 2) {
    return;
  }

  const releaseType = getInferredReleaseType(grouped);

  if (!releaseType) {
    return;
  }

  const previousTag = tags[1];
  const version = semVer.inc(previousTag.version, releaseType);

  tags[0] = {
    ...tags[0],
    tag: `${getTagPrefix(previousTag.tag)}${version}`,
    version,
  };
};

const createRelease = async (tag, changelog, exists) => {
  const isPre = flags.pre ? 'pre' : '';
  createSpinner(`Uploading ${isPre}release`);

  const { pre, publish, showUrl } = flags;

  const body = {
    owner: repoDetails.user,
    repo: repoDetails.repo,
    /* oxlint-disable camelcase */
    tag_name: tag.tag,
    target_commitish: tag.hash,
    /* oxlint-enable camelcase */
    body: changelog,
    draft: !publish,
    prerelease: pre,
  };

  if (exists) {
    body.id = exists;
  }

  let response;

  try {
    response = exists
      ? await githubConnection.rest.repos.updateRelease(body)
      : await githubConnection.rest.repos.createRelease(body);
  } catch {
    response = {};
  }

  if (!response.data) {
    console.log('\n');
    fail('Failed to upload release.');
  }

  global.spinner.succeed();
  const releaseURL = getReleaseURL(response.data, !publish);

  // Wait for the GitHub UI to render the release
  await sleep(500);

  if (!showUrl) {
    try {
      await open(releaseURL, { wait: false });
      console.log(`\n${chalk.bold('Done!')} Opened release in browser...`);

      return;
      // oxlint-disable-next-line no-empty
    } catch {}
  }

  console.log(`\n${chalk.bold('Done!')} ${releaseURL}`);
};

const printDryRun = async (tag, changelog, exists) => {
  const action = exists ? 'update' : 'create';
  const isTagOnlyWithoutBump = flags.tagOnly && args.sub.length < 1;
  const releaseKind = flags.tagOnly ? 'annotated Git tag' : 'GitHub Release';
  const target = tag.hash ? ` targeting ${tag.hash}` : '';

  if (global.spinner) {
    global.spinner.succeed();
    global.spinner = false;
  }

  console.log(
    `\n${chalk.bold('Dry run')} Would ${action} ${releaseKind} ${chalk.bold(tag.tag)}${target}`
  );

  if (isTagOnlyWithoutBump) {
    console.log('Version bump inferred from selected change types.');
  }

  if (exists) {
    console.log(`Would update existing release ${exists}`);
  }

  console.log(`\n${chalk.bold('Release notes')}\n`);
  console.log(changelog);

  if (flags.summary) {
    console.log(`\n${chalk.bold('Update summary')}\n`);
    console.log(flags.summary);
  }
};

const orderCommits = async (
  commits,
  tags,
  exists,
  publishRelease = createRelease,
  options = {}
) => {
  const questions = [];
  const predefined = {};

  const choices = getChoices(changeTypes, tags);

  // Show the latest changes first
  commits.all.reverse();

  for (const commit of commits.all) {
    const defTitle = definitions.type(commit.title, changeTypes);
    const defDescription = definitions.type(commit.description, changeTypes);

    const definition = defTitle || defDescription;

    // Firstly try to use the commit title
    let message = commit.title;

    // If it wasn't set, try the description
    if (message.length === 0) {
      const lines = commit.description.split('\n');

      for (let line of lines) {
        if (!line) {
          continue;
        }

        line = line.replace('* ', '');

        if (line.length === 0) {
          continue;
        }

        const questionExists = questions.find((question) => question.message === line);

        if (questionExists) {
          continue;
        }

        if (line.length > 1) {
          message = line;
          break;
        }
      }
    }

    // If for some reason the message is still not defined,
    // don't include it in the list
    if (message.length === 0) {
      continue;
    }

    // If a type preset was found, don't include it
    // in the list either
    if (definition) {
      predefined[commit.hash] = {
        type: definition,
        message,
      };

      continue;
    }

    // If we are skipping the questions, don't let them be included
    // in the list
    if (flags.skipQuestions) {
      predefined[commit.hash] = {
        // The type doesn't matter since it is not included in the
        // final changelog
        type: 'patch',
        message,
      };

      continue;
    }

    questions.push({
      name: commit.hash,
      message,
      type: 'select',
      choices,
    });
  }

  global.spinner.succeed();

  // Prevents the spinner from getting succeeded
  // again once new spinner gets created
  global.spinner = false;

  // By default, nothing is there yet
  let answers = {};

  if (choices && questions.length > 0) {
    console.log(`${chalk.green('!')} Please enter the type of change for each commit:\n`);

    answers = await inquirer.prompt(questions);

    for (const answer in answers) {
      if (!{}.hasOwnProperty.call(answers, answer)) {
        continue;
      }

      const type = answers[answer];
      const { message } = questions.find((question) => question.name === answer);

      answers[answer] = {
        type,
        message,
      };
    }

    // Update the spinner status
    if (choices) {
      console.log('');
    }
  }

  createSpinner('Generating the changelog');

  const results = Object.assign({}, predefined, answers);
  const grouped = groupChanges(results, changeTypes);
  applyInferredDryRunTag(tags, grouped);

  const existingRelease =
    options.releases?.find((release) => release.tag_name === tags[0].tag)?.id || exists;

  const changes = await createChangelog(
    grouped,
    commits,
    changeTypes,
    flags.skipQuestions,
    flags.hook,
    flags.showUrl
  );

  let { credits, changelog } = changes;

  if (!changelog) {
    changelog = 'Initial release';
  }

  // Apply the `release.js` file or the one that
  // was specified using the `--hook` flag
  const filtered = await applyHook(flags.hook, changelog, {
    githubConnection,
    repoDetails,
    changeTypes,
    commits,
    groupedCommits: grouped,
    authors: credits,
  });

  await publishRelease(tags[0], filtered, existingRelease);
};

const collectChanges = async (tags, exists = false, publishRelease, options = {}) => {
  createSpinner('Loading commit history');
  let commits;

  try {
    commits = await getCommits(tags, options);
  } catch (err) {
    fail(err.message);
  }

  for (const commit of commits.all) {
    if (semVer.valid(commit.title)) {
      const index = commits.all.indexOf(commit);
      commits.all.splice(index, 1);
    }
  }

  if (commits.all.length < 1) {
    fail('No changes happened since the last release.');
  }

  await orderCommits(commits, tags, exists, publishRelease, options);
};

const createTagOnlyRelease = async (release) => {
  let previousTags;

  if (flags.dryRun) {
    githubConnection = await connect(flags.showUrl);
    repoDetails = await getRepo(githubConnection);
  }

  try {
    previousTags = await getTags({
      previousTag: flags.previousTag,
    });
  } catch {
    fail('Directory is not a Git repository.');
  }

  const previousTag = flags.previousTag ? previousTags.at(-1) : previousTags.at(0);
  const tags = previousTag ? [release, previousTag] : [release];
  const publishRelease = flags.dryRun
    ? printDryRun
    : (tag, changelog) => createAnnotatedTag(tag, changelog, flags.summary);

  await collectChanges(tags, false, publishRelease, {
    includeLatest: flags.dryRun,
  });
};

const checkReleaseStatus = async (release = null) => {
  let tags;

  try {
    const unordered = await getTags({
      previousTag: flags.previousTag,
    });
    tags = unordered.sort((a, b) => new Date(b.date) - new Date(a.date));
  } catch {
    fail('Directory is not a Git repository.');
  }

  if (release) {
    const previousTag = tags.at(0);
    tags = previousTag ? [release, previousTag] : [release];
  } else if (flags.dryRun && flags.tagOnly) {
    const previousTag = tags.at(0);

    if (!previousTag) {
      fail('No tags available for release.');
    }

    tags = [
      {
        tag: 'HEAD',
        version: previousTag.version,
        hash: await getHead(),
        date: new Date(),
      },
      previousTag,
    ];
  }

  if (tags.length < 1) {
    fail('No tags available for release.');
  }

  const synced = flags.dryRun || release ? true : await branchSynced();

  if (!synced) {
    fail('Your branch needs to be up-to-date with origin.');
  }

  githubConnection = await connect(flags.showUrl);
  repoDetails = await getRepo(githubConnection);

  createSpinner('Checking if release already exists');

  let response;

  try {
    response = await githubConnection.rest.repos.listReleases({
      owner: repoDetails.user,
      repo: repoDetails.repo,
    });
  } catch (err) {
    console.error(err);
  }

  if (!response) {
    fail("Couldn't check if release exists.");
  }

  if (!response.data || response.data.length < 1) {
    await collectChanges(tags, false, flags.dryRun ? printDryRun : createRelease, {
      includeLatest: Boolean(release) || flags.tagOnly,
      releases: response.data,
    });
    return;
  }

  let existingRelease = null;

  for (const release of response.data) {
    if (release.tag_name === tags[0].tag) {
      existingRelease = release;
      break;
    }
  }

  if (!existingRelease) {
    await collectChanges(tags, false, flags.dryRun ? printDryRun : createRelease, {
      includeLatest: Boolean(release) || flags.tagOnly,
      releases: response.data,
    });
    return;
  }

  if (flags.overwrite) {
    global.spinner.text = 'Overwriting release, because it already exists';
    await collectChanges(tags, existingRelease.id, flags.dryRun ? printDryRun : undefined, {
      includeLatest: Boolean(release) || flags.tagOnly,
      releases: response.data,
    });

    return;
  }

  if (flags.dryRun) {
    await collectChanges(tags, existingRelease.id, printDryRun, {
      includeLatest: Boolean(release) || flags.tagOnly,
      releases: response.data,
    });

    return;
  }

  global.spinner.succeed();
  console.log('');

  const releaseURL = getReleaseURL(existingRelease);
  const prefix = `${chalk.red('Error!')} Release already exists`;

  if (!flags.showUrl && !flags.dryRun) {
    try {
      await open(releaseURL, { wait: false });
      console.error(`${prefix}. Opened in browser...`);

      return;
      // oxlint-disable-next-line no-empty
    } catch {}
  }

  console.error(`${prefix}: ${releaseURL}`);
  process.exit(1);
};

const main = async () => {
  flags.summary = flags.summary.trim();

  if (/[\r\n]/.test(flags.summary)) {
    fail('The update summary must fit on one line.');
  }

  if (flags.summary && !flags.tagOnly) {
    fail('The "--summary" option requires "--tag-only".');
  }

  let update = null;

  try {
    update = await checkForUpdate();
  } catch {}

  if (update) {
    console.log(
      `${chalk.bgRed('UPDATE AVAILABLE')} The latest version of \`release\` is ${update.latest}`
    );
  }

  const bumpType = args.sub;
  const argAmount = bumpType.length;
  const isBump = argAmount === 1 || (bumpType[0] === 'pre' && argAmount === 2);

  if (flags.tagOnly && !isBump && !flags.dryRun) {
    fail('The "--tag-only" option requires a version bump.');
  }

  if (isBump) {
    const allowedTypes = ['pre'];

    for (const type of changeTypes) {
      allowedTypes.push(type.handle);
    }

    const allowed = allowedTypes.includes(bumpType[0]);
    const type = bumpType[0];

    if (!allowed) {
      fail('Version type not SemVer-compatible ' + '("major", "minor", "patch" or "pre")');
    }

    if (flags.tagOnly && !flags.dryRun) {
      const synced = await branchSynced();

      if (!synced) {
        fail('Your branch needs to be up-to-date with origin.');
      }
    }

    const release = await bumpVersion(type, bumpType[1], flags.tagOnly, flags.dryRun);

    if (flags.tagOnly) {
      await createTagOnlyRelease(release);
      return;
    }

    if (flags.dryRun) {
      await checkReleaseStatus(release);
      return;
    }
  }

  await checkReleaseStatus();
};

// Let the firework start
await main();
