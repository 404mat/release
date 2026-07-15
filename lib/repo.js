// Native
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

// Utilities
import * as handleSpinner from './spinner.js';

const execGit = async (args) => {
  const { stdout } = await promisify(execFile)('git', args, {
    cwd: process.cwd(),
  });

  return stdout.trim();
};

const parseGitHubRemote = (remote) => {
  const sshMatch = /^git@github\.com:([^/]+)\/(.+?)(?:\.git)?$/.exec(remote);

  if (sshMatch) {
    return {
      user: sshMatch[1],
      repo: sshMatch[2],
    };
  }

  const { hostname, pathname } = new URL(remote);

  if (hostname !== 'github.com') {
    throw new Error('Remote origin is not hosted on GitHub.');
  }

  const [user, rawRepo] = pathname.replace(/^\//, '').split('/');

  return {
    user,
    repo: rawRepo.replace(/\.git$/, ''),
  };
};

export const getRepo = async (githubConnection) => {
  let details;

  try {
    const remote = await execGit(['config', '--get', 'remote.origin.url']);
    details = parseGitHubRemote(remote);
  } catch {
    handleSpinner.fail('Could not determine GitHub repository.');
  }

  try {
    const detailedRepo = await githubConnection.rest.repos.get({
      owner: details.user,
      repo: details.repo,
    });

    return {
      repo: details.repo,
      user: detailedRepo.data.owner.login,
    };
  } catch {
    handleSpinner.fail('Could not determine GitHub repository.');
  }
};

export const branchSynced = () =>
  Promise.all([
    execGit(['rev-parse', '--is-inside-work-tree']),
    execGit(['status', '--porcelain', '--untracked-files=no']),
    execGit(['rev-list', '--count', 'HEAD', '--not', '--remotes']),
  ])
    .then(([, status, ahead]) => status.length === 0 && Number(ahead) === 0)
    .catch(() => false);
