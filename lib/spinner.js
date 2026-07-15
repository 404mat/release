// Packages
import chalk from 'chalk';
import ora from 'ora';

export const create = (message) => {
  if (global.spinner) {
    global.spinner.succeed();
  }

  global.spinner = ora(message).start();
};

export const fail = (message) => {
  if (global.spinner) {
    global.spinner.fail();
    console.log('');
  }

  console.error(`${chalk.red('Error!')} ${message}`);
  process.exit(1);
};
