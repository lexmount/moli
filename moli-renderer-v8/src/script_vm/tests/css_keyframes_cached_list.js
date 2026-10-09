((bindings, values, label) => {
  const checks=[];
  const check=(name,fn)=>{try{const ok=fn();checks.push({name:label+': '+name,passed:ok===true,detail:ok===true?null:String(ok)});}catch(error){checks.push({name:label+': '+name,passed:false,detail:String(error)});}};
  const sheet=new values.CSSStyleSheet();sheet.replaceSync('@keyframes cached {from {left:0px} to {left:100px}}');
  const rule=sheet.cssRules[0],list=rule.cssRules,original=Object.getPrototypeOf(list);
  const getter=Object.getOwnPropertyDescriptor(bindings.CSSKeyframesRule.prototype,'cssRules').get;
  check('cached access retains original prototype',()=>{const result=getter.call(rule);return result===list && Object.getPrototypeOf(result)===original;});
  check('repeated cached access retains owner realm',()=>{const result=getter.call(rule);return result===list && result instanceof values.CSSRuleList;});
  Object.setPrototypeOf(list,null);
  try {
    check('cached access preserves author prototype changes',()=>{const result=getter.call(rule);return result===list && Object.getPrototypeOf(result)===null;});
    check('prototype change preserves native list identity and items',()=>values.CSSRuleList.prototype.item.call(list,0)===rule[0] && Object.getPrototypeOf(list)===null);
  } finally {Object.setPrototypeOf(list,original);}
  return checks;
})
