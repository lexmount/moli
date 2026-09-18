styleNestingEvents.push('script');
window.styleNestingRan = true;
window.styleNestingSheetAtRun = document.getElementById('gate').sheet !== null;
window.styleNestingCurrentScript = document.currentScript.id;
document.write('<span id="written">ok</span>');
