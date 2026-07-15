// Native
import { exec } from 'node:child_process';
import { promisify } from 'node:util';

// Packages
import semVer from 'semver';

const defaultRev = 'HEAD --first-parent `git rev-parse --abbrev-ref HEAD`';

const defaultOptions = {
  rev: defaultRev,
  previousTag: '',
};

const tagRegex = /tag:\s*([^,)]+)/g;

const runCommand = async (command) => {
  const { stdout } = await promisify(exec)(command, {
    cwd: process.cwd(),
    maxBuffer: 1024 * 1024 * 50,
  });

  return stdout;
};

const parseTags = (output) =>
  output
    .split('\n')
    .flatMap((line) => {
      const [refs, hash, date] = line.split(';');

      if (!refs || !hash || !date) {
        return [];
      }

      const tags = [...refs.matchAll(tagRegex)];

      return tags
        .map((match) => {
          const tag = match[1];
          const version = semVer.valid(tag);

          if (!version) {
            return null;
          }

          return {
            tag,
            version,
            hash: hash.trim(),
            date: new Date(date.trim()),
          };
        })
        .filter(Boolean);
    })
    .sort((a, b) => semVer.rcompare(a.version, b.version));

const getList = async (rev) => {
  const format = '--pretty="%d;%H;%ci" --decorate=short';
  const command = rev
    ? `git log --simplify-by-decoration ${format} ${rev}`
    : `git log --no-walk --tags ${format}`;

  return parseTags(await runCommand(command));
};

export default async (options = {}) => {
  const { rev, previousTag } = { ...defaultOptions, ...options };
  const tags = await getList(rev);
  const latest = tags[0];

  if (!latest) {
    return [];
  }

  const isPreviousTag =
    previousTag && previousTag.length > 0
      ? (commitVersion) => commitVersion === previousTag
      : semVer.lt;

  for (const commit of tags) {
    if (isPreviousTag(commit.version, latest.version)) {
      return [latest, commit];
    }
  }

  return [latest];
};
