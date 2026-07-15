// Native
import queryString from 'node:querystring';
import { randomBytes } from 'node:crypto';

// Packages
import { Octokit } from '@octokit/rest';
import Configstore from 'configstore';
import open from 'open';
import retry from 'async-retry';
import sleep from 'delay';

// Utilities
import pkg from '../package.json' with { type: 'json' };
import * as handleSpinner from './spinner.js';

// Initialize token storage
const config = new Configstore(pkg.name);

const createGitHubClient = (token) =>
  new Octokit({
    auth: token,
    userAgent: `Release v${pkg.version}`,
  });

const tokenAPI = (state) =>
  retry(
    async () => {
      const qs = queryString.stringify({ state });
      const authURL = process.env.RELEASE_AUTH_URL;

      if (!authURL) {
        throw new Error('RELEASE_AUTH_URL is not configured');
      }

      const res = await fetch(`${authURL}?${qs}`);

      if (res.status === 403) {
        throw new Error('Unauthorized');
      }

      const data = await res.json();

      if (data.error) {
        throw new Error(data.error);
      }

      return data.token;
    },
    {
      retries: 500,
    }
  );

const validateToken = async (token) => {
  const github = createGitHubClient(token);

  try {
    await github.rest.users.getAuthenticated();
    return true;
  } catch {
    return false;
  }
};

const loadToken = async () => {
  if (config.has('token')) {
    const fromStore = config.get('token');
    const valid = await validateToken(fromStore);

    return valid ? fromStore : false;
  }

  return false;
};

const requestToken = async (showURL) => {
  let authURL = 'https://github.com/login/oauth/authorize';

  const state = randomBytes(10).toString('hex');

  const params = {
    // oxlint-disable-next-line camelcase
    client_id: process.env.GITHUB_CLIENT_ID,
    scope: 'repo',
    state,
  };

  authURL += `?${queryString.stringify(params)}`;

  try {
    if (showURL) {
      throw new Error('No browser support');
    }

    await open(authURL, { wait: false });
  } catch {
    global.spinner.stop();
    console.log(`Please click this link to authenticate: ${authURL}`);
  }

  const token = await tokenAPI(state);
  config.set('token', token);

  return token;
};

export default async (showURL) => {
  let token = await loadToken();

  if (!token) {
    handleSpinner.create(
      showURL ? 'Retrieving authentication link' : 'Opening GitHub authentication page'
    );
    await sleep(100);

    try {
      token = await requestToken(showURL);
    } catch {
      handleSpinner.fail('Could not load token.');
    }
  }

  return createGitHubClient(token);
};
