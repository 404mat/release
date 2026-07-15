// Packages
import capitalize from 'capitalize';
import { htmlEscape } from 'escape-goat';

// Utilities
import connect from './connect.js';
import * as definitions from './definitions.js';
import * as repo from './repo.js';

const getPullRequest = async (number, showURL) => {
  const github = await connect(showURL);
  const repoDetails = await repo.getRepo(github);

  const response = await github.rest.pulls.get({
    owner: repoDetails.user,
    repo: repoDetails.repo,
    number,
  });

  return response.data;
};

const forPullRequest = async (number, showURL) => {
  let data;

  try {
    data = await getPullRequest(number, showURL);
  } catch {
    return;
  }

  if (data.user) {
    return [data.user.login];
  }

  return false;
};

const cleanCommitTitle = (title, changeTypes, doEscapeHTML) => {
  const toReplace = {
    type: definitions.type(title, changeTypes),
    ref: definitions.reference(title),
  };

  for (const definition in toReplace) {
    if (!{}.hasOwnProperty.call(toReplace, definition)) {
      continue;
    }

    const state = toReplace[definition];

    if (state) {
      title = title.replace(`(${state})`, '');
    }
  }

  if (doEscapeHTML) {
    title = htmlEscape(title);
  }

  return {
    content: capitalize(title).trim(),
    ref: toReplace.ref,
  };
};

export default async ({ hash, message }, all, changeTypes, doEscapeHTML, showURL) => {
  const title = cleanCommitTitle(message, changeTypes, doEscapeHTML);
  let credits = [];

  if (title.ref) {
    hash = title.ref;

    const rawHash = hash.split('#')[1];

    // Retrieve users that have collaborated on a change
    const collaborators = await forPullRequest(rawHash, showURL);

    if (collaborators) {
      credits = credits.concat(collaborators);
    }
  }

  return {
    text: `- ${title.content}: ${hash}\n`,
    credits,
  };
};
