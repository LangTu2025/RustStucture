from pathlib import Path
import json,collections

def typed(t,v):return {'type':t,'value':v}
def tag(v):
 if 'byte' in v:return typed('Byte',v['byte'] if v['byte']<128 else v['byte']-256)
 if 'int' in v:return typed('Int',v['int'])
 return typed('String',v['string'])
def probe(info):
 typ=info.get('flattenedPropertyType','string') or 'string'
 keys=list(info.get('flattenedValueRemaps',{}))
 value=keys[0] if keys else ('probe' if typ=='string' else '0')
 return typed({'string':'String','int':'Int','byte':'Byte'}[typ],value if typ=='string' else int(value))
cases=[];coverage=collections.Counter()
def add(file,kind,name,props,version):
 cases.append({'label':f'{file}:{kind}:{name}:{len(cases)}','input':{'name':name,'properties':props},'version':version})
 coverage[kind]+=1
for path in sorted(Path('assets/block_upgrade_schemas').glob('0*.json')):
 s=json.loads(path.read_text());f=path.name
 version=(s['maxVersionMajor']<<24)|(s['maxVersionMinor']<<16)|(s['maxVersionPatch']<<8)|s['maxVersionRevision']
 for name in s.get('renamedIds',{}):add(f,'rename_id',name,{'keep':typed('Short',7)},version)
 for name,props in s.get('addedProperties',{}).items():
  add(f,'add',name,{},version)
  add(f,'add_existing',name,{k:typed('String','do_not_overwrite') for k in props},version)
 for name,props in s.get('removedProperties',{}).items():add(f,'remove',name,{**{p:typed('String','remove') for p in props},'keep':typed('Long',9007199254740993)},version)
 for name,props in s.get('renamedProperties',{}).items():
  values={}
  for p in props:
   lookup=s.get('remappedPropertyValues',{}).get(name,{}).get(p)
   values[p]=tag(s['remappedPropertyValuesIndex'][lookup][0]['old']) if lookup else typed('Int',7)
  add(f,'rename_property',name,values,version)
 for name,info in s.get('flattenedProperties',{}).items():
  value=probe(info);add(f,'flatten_'+info.get('flattenedPropertyType','string'),name,{info['flattenedProperty']:value,'keep':typed('Byte',1)},version)
  add(f,'flatten_wrong_type',name,{info['flattenedProperty']:typed('Short',7)},version)
 for name,props in s.get('remappedPropertyValues',{}).items():
  for prop,lookup in props.items():
   for pair in s['remappedPropertyValuesIndex'][lookup]:
    value=tag(pair['old']);add(f,'value_remap',name,{prop:value,'keep':typed('Byte',1)},version)
    if value['type']=='Int':add(f,'strict_integer_type',name,{prop:typed('Short',value['value'])},version)
 for name,remaps in s.get('remappedStates',{}).items():
  for remap in remaps:
   props={k:tag(v) for k,v in (remap.get('oldState') or {}).items()}
   for p in (remap.get('copiedState') or []):props.setdefault(p,typed('String','copied'))
   info=remap.get('newFlattenedName') or {}
   if info.get('flattenedProperty'):props.setdefault(info['flattenedProperty'],probe(info))
   add(f,'state_remap',name,props,version)
   if props:
    wrong=dict(props);first=next(iter(props));wrong[first]=typed('String','mismatch')
    add(f,'state_remap_mismatch',name,wrong,version)
 # 最大版本之后必须不应用任何升级；每个schema的inclusive边界由上面的实际规则输入覆盖。
 add(f,'future_version','minecraft:stone',{'keep':typed('Int',7)},2147483647)
# 为真实触发输入补齐版本边界。
for case in list(cases):
 if case['version']!=2147483647:
  for offset in (-1,1):
   add(case['label'].split(':')[0],'version_boundary_'+str(offset),case['input']['name'],case['input']['properties'],case['version']+offset)
for path in sorted(Path('assets/block_upgrade_schemas').glob('0*.json')):
 s=json.loads(path.read_text())
 version=(s['maxVersionMajor']<<24)|(s['maxVersionMinor']<<16)|(s['maxVersionPatch']<<8)|s['maxVersionRevision']
 for name,info in s.get('flattenedProperties',{}).items():
  typ=info.get('flattenedPropertyType','string') or 'string'
  if typ=='byte':
   for value in (0,1,127,-128,-1):
    add(path.name,'flatten_byte_unsigned',name,{info['flattenedProperty']:typed('Byte',value)},version)
  if typ=='int':
   for value in (-1,0,1,15,255):
    add(path.name,'flatten_integer',name,{info['flattenedProperty']:typed('Int',value)},version)
Path('work/block-upgrade-oracle/input.json').write_text(json.dumps(cases,ensure_ascii=False,separators=(',',':'))+'\n',encoding='utf8')
Path('work/block-upgrade-oracle/coverage.json').write_text(json.dumps(dict(coverage),indent=2)+'\n',encoding='utf8')
print(len(cases),dict(coverage))

