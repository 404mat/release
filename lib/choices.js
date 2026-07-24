// Packages
import inquirer from 'inquirer';
import semVer from 'semver';

export default (changeTypes, tags) => {
  const list = [];
  let notNeeded;

  if (tags.length >= 2) {
    const releaseType = semVer.diff(tags[1].version, tags[0].version);

    switch (releaseType) {
      case 'minor':
        notNeeded = 1;
        break;
      case 'patch':
        notNeeded = 2;
        break;
      default:
        notNeeded = 0;
    }
  }

  if (notNeeded) {
    changeTypes.splice(0, notNeeded);
  }

  for (const type of changeTypes) {
    const short = type.handle;

    list.push({
      name: `${type.name} (${type.description})`,
      value: short,
      short: `(${short})`,
    });
  }

  return list.concat([
    new inquirer.Separator(),
    {
      name: 'Ignore',
      short: '(ignored)',
      value: 'ignore',
    },
  ]);
};
