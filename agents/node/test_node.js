const orionis = require('./index');

orionis.start('http://127.0.0.1:7700');

async function subTask() {
    return orionis.trace('subTask', async () => {
        console.log('Inside subTask');
        await new Promise(resolve => setTimeout(resolve, 50));
    });
}

orionis.trace('mainTask', async () => {
    console.log('Inside mainTask');
    await subTask();
});

// Give it a moment to flush before exiting
setTimeout(() => {
    console.log('Test complete');
    process.exit(0);
}, 500);
