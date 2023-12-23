"use strict";(self.webpackChunk=self.webpackChunk||[]).push([[19,579],{41832:(e,t,n)=>{n.d(t,{Z:()=>V,I:()=>x});var s=n(43001),r=n(3827),a=n.n(r),i=n(261),o=n(44530),c=n(71783);const d=-1,u=0,l=1,S=2,p=3,y=4,I=5,m={id:"measurementTracking",initial:"idle",context:{activeViewportId:null,trackedStudy:"",trackedSeries:[],ignoredSeries:[],prevTrackedStudy:"",prevTrackedSeries:[],prevIgnoredSeries:[],ignoredSRSeriesForHydration:[],isDirty:!1},states:{off:{type:"final"},idle:{entry:"clearContext",on:{TRACK_SERIES:"promptBeginTracking",SET_TRACKED_SERIES:[{target:"tracking",actions:["setTrackedStudyAndMultipleSeries","setIsDirtyToClean"]}],PROMPT_HYDRATE_SR:{target:"promptHydrateStructuredReport",cond:"hasNotIgnoredSRSeriesForHydration"},RESTORE_PROMPT_HYDRATE_SR:"promptHydrateStructuredReport",HYDRATE_SR:"hydrateStructuredReport",UPDATE_ACTIVE_VIEWPORT_ID:{actions:(0,i.f0)({activeViewportId:(e,t)=>t.activeViewportId})}}},promptBeginTracking:{invoke:{src:"promptBeginTracking",onDone:[{target:"tracking",actions:["setTrackedStudyAndSeries","setIsDirty"],cond:"shouldSetStudyAndSeries"},{target:"off",cond:"shouldKillMachine"},{target:"idle"}],onError:{target:"idle"}}},tracking:{on:{TRACK_SERIES:[{target:"promptTrackNewStudy",cond:"isNewStudy"},{target:"promptTrackNewSeries",cond:"isNewSeries"}],UNTRACK_SERIES:[{target:"tracking",actions:["removeTrackedSeries","setIsDirty"],cond:"hasRemainingTrackedSeries"},{target:"idle"}],SET_TRACKED_SERIES:[{target:"tracking",actions:["setTrackedStudyAndMultipleSeries"]}],SAVE_REPORT:"promptSaveReport",SET_DIRTY:[{target:"tracking",actions:["setIsDirty"],cond:"shouldSetDirty"},{target:"tracking"}]}},promptTrackNewSeries:{invoke:{src:"promptTrackNewSeries",onDone:[{target:"tracking",actions:["addTrackedSeries","setIsDirty"],cond:"shouldAddSeries"},{target:"tracking",actions:["discardPreviouslyTrackedMeasurements","setTrackedStudyAndSeries","setIsDirty"],cond:"shouldSetStudyAndSeries"},{target:"promptSaveReport",cond:"shouldPromptSaveReport"},{target:"tracking"}],onError:{target:"idle"}}},promptTrackNewStudy:{invoke:{src:"promptTrackNewStudy",onDone:[{target:"tracking",actions:["discardPreviouslyTrackedMeasurements","setTrackedStudyAndSeries","setIsDirty"],cond:"shouldSetStudyAndSeries"},{target:"tracking",actions:["ignoreSeries"],cond:"shouldAddIgnoredSeries"},{target:"promptSaveReport",cond:"shouldPromptSaveReport"},{target:"tracking"}],onError:{target:"idle"}}},promptSaveReport:{invoke:{src:"promptSaveReport",onDone:[{target:"idle",actions:["clearAllMeasurements","showStructuredReportDisplaySetInActiveViewport"],cond:"shouldSaveAndContinueWithSameReport"},{target:"tracking",actions:["discardPreviouslyTrackedMeasurements","setTrackedStudyAndSeries"],cond:"shouldSaveAndStartNewReport"},{target:"tracking"}],onError:{target:"idle"}}},promptHydrateStructuredReport:{invoke:{src:"promptHydrateStructuredReport",onDone:[{target:"tracking",actions:["setTrackedStudyAndMultipleSeries","jumpToFirstMeasurementInActiveViewport","setIsDirtyToClean"],cond:"shouldHydrateStructuredReport"},{target:"idle",actions:["ignoreHydrationForSRSeries"],cond:"shouldIgnoreHydrationForSR"}],onError:{target:"idle"}}},hydrateStructuredReport:{invoke:{src:"hydrateStructuredReport",onDone:[{target:"tracking",actions:["setTrackedStudyAndMultipleSeries","jumpToFirstMeasurementInActiveViewport","setIsDirtyToClean"]}],onError:{target:"idle"}}}},strict:!0},g={services:{promptBeginTracking:(e,t)=>{},promptTrackNewStudy:(e,t)=>{},promptTrackNewSeries:(e,t)=>{}},actions:{discardPreviouslyTrackedMeasurements:(e,t)=>{console.log("discardPreviouslyTrackedMeasurements: not implemented")},clearAllMeasurements:(e,t)=>{console.log("clearAllMeasurements: not implemented")},jumpToFirstMeasurementInActiveViewport:(e,t)=>{console.warn("jumpToFirstMeasurementInActiveViewport: not implemented")},showStructuredReportDisplaySetInActiveViewport:(e,t)=>{console.warn("showStructuredReportDisplaySetInActiveViewport: not implemented")},clearContext:(0,i.f0)({trackedStudy:"",trackedSeries:[],ignoredSeries:[],prevTrackedStudy:"",prevTrackedSeries:[],prevIgnoredSeries:[]}),setTrackedStudyAndSeries:(0,i.f0)(((e,t)=>({prevTrackedStudy:e.trackedStudy,prevTrackedSeries:e.trackedSeries.slice(),prevIgnoredSeries:e.ignoredSeries.slice(),trackedStudy:t.data.StudyInstanceUID,trackedSeries:[t.data.SeriesInstanceUID],ignoredSeries:[]}))),setTrackedStudyAndMultipleSeries:(0,i.f0)(((e,t)=>{const n=t.StudyInstanceUID||t.data.StudyInstanceUID,s=t.SeriesInstanceUIDs||t.data.SeriesInstanceUIDs;return{prevTrackedStudy:e.trackedStudy,prevTrackedSeries:e.trackedSeries.slice(),prevIgnoredSeries:e.ignoredSeries.slice(),trackedStudy:n,trackedSeries:[...e.trackedSeries,...s],ignoredSeries:[]}})),setIsDirtyToClean:(0,i.f0)(((e,t)=>({isDirty:!1}))),setIsDirty:(0,i.f0)(((e,t)=>({isDirty:!0}))),ignoreSeries:(0,i.f0)(((e,t)=>({prevIgnoredSeries:[...e.ignoredSeries],ignoredSeries:[...e.ignoredSeries,t.data.SeriesInstanceUID]}))),ignoreHydrationForSRSeries:(0,i.f0)(((e,t)=>({ignoredSRSeriesForHydration:[...e.ignoredSRSeriesForHydration,t.data.srSeriesInstanceUID]}))),addTrackedSeries:(0,i.f0)(((e,t)=>({prevTrackedSeries:[...e.trackedSeries],trackedSeries:[...e.trackedSeries,t.data.SeriesInstanceUID]}))),removeTrackedSeries:(0,i.f0)(((e,t)=>({prevTrackedSeries:e.trackedSeries.slice().filter((e=>e!==t.SeriesInstanceUID)),trackedSeries:e.trackedSeries.slice().filter((e=>e!==t.SeriesInstanceUID))})))},guards:{shouldSetDirty:(e,t)=>void 0===t.SeriesInstanceUID||e.trackedSeries.includes(t.SeriesInstanceUID),shouldKillMachine:(e,t)=>t.data&&t.data.userResponse===d,shouldAddSeries:(e,t)=>t.data&&t.data.userResponse===S,shouldSetStudyAndSeries:(e,t)=>t.data&&t.data.userResponse===p,shouldAddIgnoredSeries:(e,t)=>t.data&&t.data.userResponse===y,shouldPromptSaveReport:(e,t)=>t.data&&t.data.userResponse===l,shouldIgnoreHydrationForSR:(e,t)=>t.data&&t.data.userResponse===u,shouldSaveAndContinueWithSameReport:(e,t)=>t.data&&t.data.userResponse===l&&!0===t.data.isBackupSave,shouldSaveAndStartNewReport:(e,t)=>t.data&&t.data.userResponse===l&&!1===t.data.isBackupSave,shouldHydrateStructuredReport:(e,t)=>t.data&&t.data.userResponse===I,hasRemainingTrackedSeries:(e,t)=>e.trackedSeries.length>1||!e.trackedSeries.includes(t.SeriesInstanceUID),hasNotIgnoredSRSeriesForHydration:(e,t)=>!e.ignoredSRSeriesForHydration.includes(t.SeriesInstanceUID),isNewStudy:(e,t)=>!e.ignoredSeries.includes(t.SeriesInstanceUID)&&e.trackedStudy!==t.StudyInstanceUID,isNewSeries:(e,t)=>!e.ignoredSeries.includes(t.SeriesInstanceUID)&&!e.trackedSeries.includes(t.SeriesInstanceUID)}},D={NO_NEVER:-1,CANCEL:0,CREATE_REPORT:1,ADD_SERIES:2,SET_STUDY_AND_SERIES:3};const E=function(e,t,n){let{servicesManager:s,extensionManager:r}=e;const{uiViewportDialogService:a}=s.services,{viewportId:i,StudyInstanceUID:o,SeriesInstanceUID:d}=n;return new Promise((async function(e,t){let n=await function(e,t){return new Promise((function(n,s){const r="Track measurements for this series?",a=[{id:"prompt-begin-tracking-cancel",type:c.LZ.dt.secondary,text:"No",value:D.CANCEL},{id:"prompt-begin-tracking-no-do-not-ask-again",type:c.LZ.dt.secondary,text:"No, do not ask again",value:D.NO_NEVER},{id:"prompt-begin-tracking-yes",type:c.LZ.dt.primary,text:"Yes",value:D.SET_STUDY_AND_SERIES}],i=t=>{e.hide(),n(t)};e.show({viewportId:t,id:"measurement-tracking-prompt-begin-tracking",type:"info",message:r,actions:a,onSubmit:i,onOutsideClick:()=>{e.hide(),n(D.CANCEL)}})}))}(a,i);e({userResponse:n,StudyInstanceUID:o,SeriesInstanceUID:d,viewportId:i})}))},v={NO_NEVER:-1,CANCEL:0,CREATE_REPORT:1,ADD_SERIES:2,SET_STUDY_AND_SERIES:3,NO_NOT_FOR_SERIES:4};const R=function(e,t,n){let{servicesManager:s,extensionManager:r}=e;const{UIViewportDialogService:a}=s.services,{viewportId:i,StudyInstanceUID:o,SeriesInstanceUID:d}=n;return new Promise((async function(e,n){let s=await function(e,t){return new Promise((function(n,s){const r="Do you want to add this measurement to the existing report?",a=[{type:c.LZ.dt.secondary,text:"Cancel",value:v.CANCEL},{type:c.LZ.dt.primary,text:"Create new report",value:v.CREATE_REPORT},{type:c.LZ.dt.primary,text:"Add to existing report",value:v.ADD_SERIES}],i=t=>{e.hide(),n(t)};e.show({viewportId:t,type:"info",message:r,actions:a,onSubmit:i,onOutsideClick:()=>{e.hide(),n(v.CANCEL)}})}))}(a,i);s===v.CREATE_REPORT&&(s=t.isDirty?await function(e,t){return new Promise((function(n,s){const r="You have existing tracked measurements. What would you like to do with your existing tracked measurements?",a=[{type:"cancel",text:"Cancel",value:v.CANCEL},{type:"secondary",text:"Save",value:v.CREATE_REPORT},{type:"primary",text:"Discard",value:v.SET_STUDY_AND_SERIES}],i=t=>{e.hide(),n(t)};e.show({viewportId:t,type:"warning",message:r,actions:a,onSubmit:i,onOutsideClick:()=>{e.hide(),n(v.CANCEL)}})}))}(a,i):v.SET_STUDY_AND_SERIES),e({userResponse:s,StudyInstanceUID:o,SeriesInstanceUID:d,viewportId:i,isBackupSave:!1})}))},f={NO_NEVER:-1,CANCEL:0,CREATE_REPORT:1,ADD_SERIES:2,SET_STUDY_AND_SERIES:3,NO_NOT_FOR_SERIES:4};const T=function(e,t,n){let{servicesManager:s,extensionManager:r}=e;const{UIViewportDialogService:a}=s.services,{viewportId:i,StudyInstanceUID:o,SeriesInstanceUID:c}=n;return new Promise((async function(e,n){let s=await function(e,t){return new Promise((function(n,s){const r="Track measurements for this series?",a=[{type:"cancel",text:"No",value:f.CANCEL},{type:"secondary",text:"No, do not ask again for this series",value:f.NO_NOT_FOR_SERIES},{type:"primary",text:"Yes",value:f.SET_STUDY_AND_SERIES}],i=t=>{e.hide(),n(t)};e.show({viewportId:t,type:"info",message:r,actions:a,onSubmit:i,onOutsideClick:()=>{e.hide(),n(f.CANCEL)}})}))}(a,i);s===f.SET_STUDY_AND_SERIES&&(s=t.isDirty?await function(e,t){return new Promise((function(n,s){const r="Measurements cannot span across multiple studies. Do you want to save your tracked measurements?",a=[{type:"cancel",text:"Cancel",value:f.CANCEL},{type:"secondary",text:"No, discard previously tracked series & measurements",value:f.SET_STUDY_AND_SERIES},{type:"primary",text:"Yes",value:f.CREATE_REPORT}],i=t=>{e.hide(),n(t)};e.show({viewportId:t,type:"warning",message:r,actions:a,onSubmit:i,onOutsideClick:()=>{e.hide(),n(f.CANCEL)}})}))}(a,i):f.SET_STUDY_AND_SERIES),e({userResponse:s,StudyInstanceUID:o,SeriesInstanceUID:c,viewportId:i,isBackupSave:!1})}))};var k=n(56342);const h=4700;const w={NO_NEVER:-1,CANCEL:0,CREATE_REPORT:1,ADD_SERIES:2,SET_STUDY_AND_SERIES:3,NO_NOT_FOR_SERIES:4};const U=function(e,t,n){let{servicesManager:s,commandsManager:r,extensionManager:a}=e;const{uiDialogService:i,measurementService:o,displaySetService:c}=s.services,d=void 0===n.viewportId?n.data.viewportId:n.viewportId,u=void 0===n.isBackupSave?n.data.isBackupSave:n.isBackupSave,l=n?.data?.StudyInstanceUID,S=n?.data?.SeriesInstanceUID,{trackedStudy:p,trackedSeries:y}=t;let I;return new Promise((async function(e,t){const n=await(0,k.createReportDialogPrompt)(i,{extensionManager:a});if(n.action===w.CREATE_REPORT){const e=a.getDataSources()[0],t=o.getMeasurements().filter((e=>p===e.referenceStudyUID&&y.includes(e.referenceSeriesUID))),i=void 0===n.value||""===n.value?"Research Derived Series":n.value,d=function(e){const t=e.getActiveDisplaySets().filter((e=>"SR"===e.Modality)).map((e=>e.SeriesNumber));return Math.max(...t,h)+1}(c),u=async()=>r.runCommand("storeMeasurements",{measurementData:t,dataSource:e,additionalFindingTypes:["ArrowAnnotate"],options:{SeriesDescription:i,SeriesNumber:d}},"CORNERSTONE_STRUCTURED_REPORT");I=await(0,k.createReportAsync)({servicesManager:s,getReport:u})}else n.action;e({userResponse:n.action,createdDisplaySetInstanceUIDs:I,StudyInstanceUID:l,SeriesInstanceUID:S,viewportId:d,isBackupSave:u})}))};var M=n(42170);const A={NO_NEVER:-1,CANCEL:0,CREATE_REPORT:1,ADD_SERIES:2,SET_STUDY_AND_SERIES:3,NO_NOT_FOR_SERIES:4,HYDRATE_REPORT:5};const C=function(e,t,n){let{servicesManager:s,extensionManager:r,appConfig:a}=e;const{uiViewportDialogService:i,displaySetService:o}=s.services,{viewportId:d,displaySetInstanceUID:u}=n,l=o.getDisplaySetByUID(u);return new Promise((async function(e,t){const o=await function(e,t){return new Promise((function(n,s){const r="Do you want to continue tracking measurements for this study?",a=[{type:c.LZ.dt.secondary,text:"No",value:A.CANCEL},{type:c.LZ.dt.primary,text:"Yes",value:A.HYDRATE_REPORT}],i=t=>{e.hide(),n(t)};e.show({viewportId:t,type:"info",message:r,actions:a,onSubmit:i,onOutsideClick:()=>{e.hide(),n(A.CANCEL)}})}))}(i,d);let S,p;if(o===A.HYDRATE_REPORT){console.warn("!! HYDRATING STRUCTURED REPORT");const e=(0,M.hydrateStructuredReport)({servicesManager:s,extensionManager:r,appConfig:a},u);S=e.StudyInstanceUID,p=e.SeriesInstanceUIDs}e({userResponse:o,displaySetInstanceUID:n.displaySetInstanceUID,srSeriesInstanceUID:l.SeriesInstanceUID,viewportId:d,StudyInstanceUID:S,SeriesInstanceUIDs:p})}))};const b=function(e,t,n){let{servicesManager:s,extensionManager:r}=e;const{displaySetService:a}=s.services,{viewportId:i,displaySetInstanceUID:o}=n,c=a.getDisplaySetByUID(o);return new Promise(((e,t)=>{const a=(0,M.hydrateStructuredReport)({servicesManager:s,extensionManager:r},o),d=a.StudyInstanceUID,u=a.SeriesInstanceUIDs;e({displaySetInstanceUID:n.displaySetInstanceUID,srSeriesInstanceUID:c.SeriesInstanceUID,viewportId:i,StudyInstanceUID:d,SeriesInstanceUIDs:u})}))};var N=n(62657);const _=s.createContext();_.displayName="TrackedMeasurementsContext";const x=()=>(0,s.useContext)(_),P="@ohif/extension-cornerstone-dicom-sr.sopClassHandlerModule.dicom-sr";function O(e,t){let{servicesManager:n,commandsManager:r,extensionManager:a}=e,{children:d}=t;const[u]=(0,N.M)(),[l,S]=(0,c.O_)(),{activeViewportId:p,viewports:y}=l,{measurementService:I,displaySetService:D}=n.services,v=Object.assign({},g);v.actions=Object.assign({},v.actions,{jumpToFirstMeasurementInActiveViewport:(e,t)=>{const{trackedStudy:n,trackedSeries:s,activeViewportId:r}=e,a=I.getMeasurements().filter((e=>n===e.referenceStudyUID&&s.includes(e.referenceSeriesUID)));console.log("jumping to measurement reset viewport",r,a[0]);const i=a[0].displaySetInstanceUID,o=D.getDisplaySetByUID(i).images,c=o[0].imageId.startsWith("volumeId"),d=a[0].data;let u=0;!c&&d&&(u=o.findIndex((e=>{const t=Object.keys(d)[0].substring(8);return e.imageId===t})),-1===u&&(console.warn("Could not find image index for tracked measurement, using 0"),u=0)),S.setDisplaySetsForViewport({viewportId:r,displaySetInstanceUIDs:[i],viewportOptions:{initialImageOptions:{index:u}}})},showStructuredReportDisplaySetInActiveViewport:(e,t)=>{if(t.data.createdDisplaySetInstanceUIDs.length>0){const e=t.data.createdDisplaySetInstanceUIDs[0];S.setDisplaySetsForViewport({viewportId:t.data.viewportId,displaySetInstanceUIDs:[e]})}},discardPreviouslyTrackedMeasurements:(e,t)=>{const n=I.getMeasurements().filter((t=>e.prevTrackedSeries.includes(t.referenceSeriesUID))).map((e=>e.id));for(let e=0;e<n.length;e++)I.remove(n[e])},clearAllMeasurements:(e,t)=>{const n=I.getMeasurements().map((e=>e.uid));for(let e=0;e<n.length;e++)I.remove(n[e])}}),v.services=Object.assign({},v.services,{promptBeginTracking:E.bind(null,{servicesManager:n,extensionManager:a,appConfig:u}),promptTrackNewSeries:R.bind(null,{servicesManager:n,extensionManager:a,appConfig:u}),promptTrackNewStudy:T.bind(null,{servicesManager:n,extensionManager:a,appConfig:u}),promptSaveReport:U.bind(null,{servicesManager:n,commandsManager:r,extensionManager:a,appConfig:u}),promptHydrateStructuredReport:C.bind(null,{servicesManager:n,extensionManager:a,appConfig:u}),hydrateStructuredReport:b.bind(null,{servicesManager:n,extensionManager:a,appConfig:u})});const f=(0,i.J)(m,v),[k,h]=(0,o.eO)(f);return(0,s.useEffect)((()=>{h("UPDATE_ACTIVE_VIEWPORT_ID",{activeViewportId:p})}),[p,h]),(0,s.useEffect)((()=>{if(y.size>0){const e=y.get(p);if(!e||!e?.displaySetInstanceUIDs?.length)return;const{displaySetService:t}=n.services,s=t.getDisplaySetByUID(e.displaySetInstanceUIDs[0]);if(!s)return;s.SOPClassHandlerId===P&&!s.isLoaded&&s.load&&s.load(),s.SOPClassHandlerId===P&&!0===s.isRehydratable&&(console.log("sending event...",k),h("PROMPT_HYDRATE_SR",{displaySetInstanceUID:s.displaySetInstanceUID,SeriesInstanceUID:s.SeriesInstanceUID,viewportId:p}))}}),[p,h,n.services,y]),s.createElement(_.Provider,{value:[k,h]},d)}O.propTypes={children:a().oneOf([a().func,a().node]),servicesManager:a().object.isRequired,commandsManager:a().object.isRequired,extensionManager:a().object.isRequired,appConfig:a().object};const V=function(e){let{servicesManager:t,extensionManager:n,commandsManager:s}=e;const r=O.bind(null,{servicesManager:t,extensionManager:n,commandsManager:s});return[{name:"TrackedMeasurementsContext",context:_,provider:r}]}},28030:(e,t,n)=>{n.r(t),n.d(t,{default:()=>P});var s=n(41832),r=n(43001),a=n(3827),i=n.n(a),o=n(62474),c=n(69190),d=n(71771),u=n(71783);const{formatDate:l}=d.utils;function S(e){let{servicesManager:t,getImageSrc:n,getStudiesForPatientByMRN:a,requestDisplaySetCreationForStudy:i,dataSource:d}=e;const{displaySetService:S,uiDialogService:p,hangingProtocolService:I,uiNotificationService:m}=t.services,g=(0,o.s0)(),{t:D}=(0,c.$G)("Common"),{StudyInstanceUIDs:E}=(0,u.zG)(),[{activeViewportId:v,viewports:R},f]=(0,u.O_)(),[T,k]=(0,s.I)(),[h,w]=(0,r.useState)("primary"),[U,M]=(0,r.useState)([...E]),[A,C]=(0,r.useState)([]),[b,N]=(0,r.useState)([]),[_,x]=(0,r.useState)({}),[P,O]=(0,r.useState)(null),V=R.get(v)?.displaySetInstanceUIDs,{trackedSeries:L}=T.context;(0,r.useEffect)((()=>{E.forEach((e=>async function(e){const t=await d.query.studies.search({studyInstanceUid:e});if(!t?.length)throw g("/notfoundstudy","_self"),new Error("Invalid study URL");let n=t;try{n=await a(t)}catch(e){console.warn(e)}const s=n.map((e=>({AccessionNumber:e.accession,StudyDate:e.date,StudyDescription:e.description,NumInstances:e.instances,ModalitiesInStudy:e.modalities,PatientID:e.mrn,PatientName:e.patientName,StudyInstanceUID:e.studyInstanceUid,StudyTime:e.time}))).map((e=>({studyInstanceUid:e.StudyInstanceUID,date:l(e.StudyDate)||D("NoStudyDate"),description:e.StudyDescription,modalities:e.ModalitiesInStudy,numInstances:e.NumInstances})));C((e=>{const t=[...e];for(const n of s)e.find((e=>e.studyInstanceUid===n.studyInstanceUid))||t.push(n);return t}))}(e)))}),[E,a]),(0,r.useEffect)((()=>{const e=S.activeDisplaySets;e.length&&e.forEach((async e=>{const t={},s=S.getDisplaySetByUID(e.displaySetInstanceUID),r=d.getImageIdsForDisplaySet(s),a=r[Math.floor(r.length/2)];a&&!s?.unsupported&&(t[e.displaySetInstanceUID]=await n(a),x((e=>({...e,...t}))))}))}),[S,d,n]),(0,r.useEffect)((()=>{const e=S.activeDisplaySets;if(!e.length)return;const t=y(e,_,L,R,f,d,S,p,m);N(t)}),[S.activeDisplaySets,L,R,d,_]),(0,r.useEffect)((()=>{const e=S.subscribe(S.EVENTS.DISPLAY_SETS_ADDED,(e=>{const{displaySetsAdded:t,options:s}=e;t.forEach((async e=>{const t=e.displaySetInstanceUID,r={},a=S.getDisplaySetByUID(t);if(a?.unsupported)return;s.madeInClient&&O(t);const i=d.getImageIdsForDisplaySet(a),o=i[Math.floor(i.length/2)];o&&(r[t]=await n(o),x((e=>({...e,...r}))))}))}));return()=>{e.unsubscribe()}}),[S,d,n,_,L,R]),(0,r.useEffect)((()=>{const e=S.subscribe(S.EVENTS.DISPLAY_SETS_CHANGED,(e=>{const t=y(e,_,L,R,f,d,S,p,m);N(t)})),t=S.subscribe(S.EVENTS.DISPLAY_SET_SERIES_METADATA_INVALIDATED,(()=>{const e=y(S.getActiveDisplaySets(),_,L,R,f,d,S,p,m);N(e)}));return()=>{e.unsubscribe(),t.unsubscribe()}}),[_,L,R,d,S]);const F=function(e,t,n,s){const r=[],a=[],i=[];t.forEach((t=>{const o=n.filter((e=>e.StudyInstanceUID===t.studyInstanceUid)),c=s.getDisplaySetSortFunction();o.sort(c);const d=Object.assign({},t,{displaySets:o});e.includes(t.studyInstanceUid)?(r.push(d),i.push(d)):(a.push(d),i.push(d))}));const o=(e,t)=>{const n=Date.parse(e);return Date.parse(t)-n},c=[{name:"primary",label:"Primary",studies:r.sort(((e,t)=>o(e.date,t.date)))},{name:"recent",label:"Recent",studies:a.sort(((e,t)=>o(e.date,t.date)))},{name:"all",label:"All",studies:i.sort(((e,t)=>o(e.date,t.date)))}];return c}(E,A,b,I);return(0,r.useEffect)((()=>{if(P){const e=P,t=document.getElementById(`thumbnail-${e}`);t&&"function"==typeof t.scrollIntoView&&(t.scrollIntoView({behavior:"smooth"}),O(null))}}),[P,U,h]),(0,r.useEffect)((()=>{if(!P)return;const e=function(e,t){for(let n=0;n<t.length;n++){const{studies:s}=t[n];for(let r=0;r<s.length;r++){const{displaySets:a}=s[r];for(let i=0;i<a.length;i++){if(a[i].displaySetInstanceUID===e)return{tabName:t[n].name,StudyInstanceUID:s[r].studyInstanceUid}}}}}(P,F);if(!e)return void console.warn("jumpToThumbnail: displaySet thumbnail not found.");const{tabName:t,StudyInstanceUID:n}=e;w(t);if(!U.includes(n)){const e=[...U,n];M(e)}}),[U,P,F]),r.createElement(u.eX,{tabs:F,servicesManager:t,activeTabName:h,expandedStudyInstanceUIDs:U,onClickStudy:function(e){const t=U.includes(e),n=t?[...U.filter((t=>t!==e))]:[...U,e];if(M(n),!t){i(S,e,!0)}},onClickTab:e=>{w(e)},onClickUntrack:e=>{const t=S.getDisplaySetByUID(e);k("UNTRACK_SERIES",{SeriesInstanceUID:t.SeriesInstanceUID})},onClickThumbnail:()=>{},onDoubleClickThumbnail:e=>{let t=[];const n=v;try{t=I.getViewportsRequireUpdate(n,e)}catch(e){console.warn(e),m.show({title:"Thumbnail Double Click",message:"The selected display sets could not be added to the viewport due to a mismatch in the Hanging Protocol rules.",type:"info",duration:3e3})}f.setDisplaySetsForViewports(t)},activeDisplaySetInstanceUIDs:V})}S.propTypes={servicesManager:i().object.isRequired,dataSource:i().shape({getImageIdsForDisplaySet:i().func.isRequired}).isRequired,getImageSrc:i().func.isRequired,getStudiesForPatientByMRN:i().func.isRequired,requestDisplaySetCreationForStudy:i().func.isRequired};const p=S;function y(e,t,n,s,a,i,o,c,d){const S=[],p=[];return e.filter((e=>!e.excludeFromThumbnailBrowser)).forEach((e=>{const y=t[e.displaySetInstanceUID],m=function(e){if(I.includes(e.Modality)||e?.unsupported)return"thumbnailNoImage";return"thumbnailTracked"}(e),g=a.getNumViewportPanes(),D=[];1!==g&&s.forEach((t=>{t?.displaySetInstanceUIDs?.includes(e.displaySetInstanceUID)&&D.push(t.viewportLabel)}));const E="thumbnailTracked"===m?S:p,{displaySetInstanceUID:v}=e,R={displaySetInstanceUID:v,description:e.SeriesDescription,seriesNumber:e.SeriesNumber,modality:e.Modality,seriesDate:l(e.SeriesDate),numInstances:e.numImageFrames,countIcon:e.countIcon,messages:e.messages,StudyInstanceUID:e.StudyInstanceUID,componentType:m,imageSrc:y,dragData:{type:"displayset",displaySetInstanceUID:v},isTracked:n.includes(e.SeriesInstanceUID),isHydratedForDerivedDisplaySet:e.isHydrated,viewportIdentificator:D};"thumbnailNoImage"===m&&(i.reject&&i.reject.series?(R.canReject=!e?.unsupported,R.onReject=()=>{c.create({id:"ds-reject-sr",centralize:!0,isDraggable:!1,showOverlay:!0,content:u.Vq,contentProps:{title:"Delete Report",body:()=>r.createElement("div",{className:"bg-primary-dark p-4 text-white"},r.createElement("p",null,"Are you sure you want to delete this report?"),r.createElement("p",{className:"mt-2"},"This action cannot be undone.")),actions:[{id:"cancel",text:"Cancel",type:u.LZ.dt.secondary},{id:"yes",text:"Yes",type:u.LZ.dt.primary,classes:["reject-yes-button"]}],onClose:()=>c.dismiss({id:"ds-reject-sr"}),onShow:()=>{document.querySelector(".reject-yes-button").focus()},onSubmit:async t=>{let{action:n}=t;switch(n.id){case"yes":try{await i.reject.series(e.StudyInstanceUID,e.SeriesInstanceUID),o.deleteDisplaySet(v),c.dismiss({id:"ds-reject-sr"}),d.show({title:"Delete Report",message:"Report deleted successfully",type:"success"})}catch(e){c.dismiss({id:"ds-reject-sr"}),d.show({title:"Delete Report",message:"Failed to delete report",type:"error"})}break;case"cancel":c.dismiss({id:"ds-reject-sr"})}}}})}):R.canReject=!1),E.push(R)})),[...S,...p]}const I=["SR","SEG","SM","RTSTRUCT","RTPLAN","RTDOSE","DOC","OT"];const m=function(e,t){return new Promise(((n,s)=>{const r=document.createElement("canvas");e.utilities.loadImageToCanvas({canvas:r,imageId:t}).then((e=>{n(r.toDataURL())})).catch(s)}))};const g=function(e,t,n,s){t.activeDisplaySets.some((e=>e.StudyInstanceUID===n))||e.retrieve.series.metadata({StudyInstanceUID:n,madeInClient:s})};function D(e){let{commandsManager:t,extensionManager:n,servicesManager:s}=e;const a=n.getActiveDataSource()[0],i=function(e){const t=e.getModuleEntry("@ohif/extension-default.utilityModule.common"),{getStudiesForPatientByMRN:n}=t.exports;return n}(n),o=i.bind(null,a),c=function(e){const t=e.getModuleEntry("@ohif/extension-cornerstone.utilityModule.common");try{const{cornerstone:e}=t.exports.getCornerstoneLibraries();return m.bind(null,e)}catch(e){throw new Error("Required command not found")}}(n),d=g.bind(null,a);return r.createElement(p,{servicesManager:s,dataSource:a,getImageSrc:c,getStudiesForPatientByMRN:o,requestDisplaySetCreationForStudy:d})}D.propTypes={commandsManager:i().object.isRequired,extensionManager:i().object.isRequired,servicesManager:i().object.isRequired};const E=D;var v=n(10800);function R(e){let{onExportClick:t,onCreateReportClick:n,disabled:s}=e;const{t:a}=(0,c.$G)("MeasurementTable");return r.createElement(r.Fragment,null,r.createElement(u.zx,{onClick:t,disabled:s,type:u.LZ.dt.secondary,size:u.LZ.dp.small},a("Export")),r.createElement(u.zx,{className:"ml-2",onClick:n,type:u.LZ.dt.secondary,size:u.LZ.dp.small,disabled:s},a("Create Report")))}R.propTypes={onExportClick:i().func,onCreateReportClick:i().func,disabled:i().bool},R.defaultProps={onExportClick:()=>alert("Export"),onCreateReportClick:()=>alert("Create Report"),disabled:!1};const f=R;var T=n(8324),k=n.n(T);const{downloadCSVReport:h}=d.utils,{formatDate:w}=d.utils,U={key:void 0,date:"",modality:"",description:""};function M(e){let{servicesManager:t,extensionManager:n}=e;const[a]=(0,u.O_)(),[i,o]=(0,r.useState)(Date.now().toString()),c=(0,v.N)(i,200),{measurementService:l,uiDialogService:S,displaySetService:p}=t.services,[y,I]=(0,s.I)(),{trackedStudy:m,trackedSeries:g}=y.context,[D,E]=(0,r.useState)(U),[R,T]=(0,r.useState)([]),M=(0,r.useRef)(null);(0,r.useEffect)((()=>{const e=l.getMeasurements().filter((e=>m===e.referenceStudyUID&&g.includes(e.referenceSeriesUID))).map((e=>function(e,t,n){const{referenceStudyUID:s,referenceSeriesUID:r,SOPInstanceUID:a}=e,i=(d.DicomMetadataStore.getInstance(s,r,a),n.getDisplaySetsForSeries(r));if(!i[0]||!i[0].images)throw new Error('The tracked measurements panel should only be tracking "stack" displaySets.');const{displayText:o,uid:c,label:u,type:l,selected:S,findingSites:p,finding:y}=e,I=p?.[0],m=u||y?.text||I?.text||"(empty)";let g=o||[];if(p){const e=[];p.forEach((t=>{t?.text!==m&&e.push(t.text)})),g=[...e,...g]}y&&y?.text!==m&&(g=[y.text,...g]);return{uid:c,label:m,baseLabel:u,measurementType:l,displayText:g,baseDisplayText:o,isActive:S,finding:y,findingSites:p}}(e,l.VALUE_TYPES,p)));T(e)}),[l,m,g,c]);const A=async()=>{if(y.matches("tracking")){const e=m,t=d.DicomMetadataStore.getStudy(e),n=t.series[0].instances[0],{StudyDate:s,StudyDescription:r}=n,a=new Set;t.series.forEach((e=>{g.includes(e.SeriesInstanceUID)&&a.add(e.instances[0].Modality)}));const i=Array.from(a).join("/");D.key!==e&&E({key:e,date:s,modality:i,description:r})}else""!==m&&void 0!==m||E(U)};(0,r.useEffect)((()=>{A()}),[D.key,y,m,A]),(0,r.useEffect)((()=>{const e=l.EVENTS.MEASUREMENT_ADDED,t=l.EVENTS.RAW_MEASUREMENT_ADDED,n=l.EVENTS.MEASUREMENT_UPDATED,s=l.EVENTS.MEASUREMENT_REMOVED,r=l.EVENTS.MEASUREMENTS_CLEARED,a=[];return[e,t,n,s,r].forEach((t=>{a.push(l.subscribe(t,(()=>{o(Date.now().toString()),t===e&&k()((()=>{M.current.scrollTop=M.current.scrollHeight}),300)()})).unsubscribe)})),()=>{a.forEach((e=>{e()}))}}),[l,I]);const C=e=>{let{uid:t,isActive:n}=e;l.jumpToMeasurement(a.activeViewportId,t),N({uid:t,isActive:n})},b=e=>{let{uid:t,isActive:n}=e;const s=l.getMeasurement(t);C({uid:t,isActive:n});const a=e=>{let{action:n,value:r}=e;if("save"===n.id)l.update(t,{...s,...r},!0);S.dismiss({id:"enter-annotation"})};S.create({id:"enter-annotation",centralize:!0,isDraggable:!1,showOverlay:!0,content:u.Vq,contentProps:{title:"Annotation",noCloseButton:!0,value:{label:s.label||""},body:e=>{let{value:t,setValue:n}=e;return r.createElement(u.II,{label:"Enter your annotation",labelClassName:"text-white grow text-[14px] leading-[1.2]",autoFocus:!0,id:"annotation",className:"border-primary-main bg-black",type:"text",value:t.label,onChange:e=>{e.persist(),n((t=>({...t,label:e.target.value})))},onKeyPress:e=>{"Enter"===e.key&&a({value:t,action:{id:"save"}})}})},actions:[{id:"cancel",text:"Cancel",type:u.LZ.dt.secondary},{id:"save",text:"Save",type:u.LZ.dt.primary}],onSubmit:a}})},N=e=>{let{uid:t,isActive:n}=e;if(!n){const e=[...R],n=e.find((e=>e.uid===t));e.forEach((e=>e.isActive=e.uid===t)),n.isActive=!0,T(e)}},_=R.filter((e=>e.measurementType!==l.VALUE_TYPES.POINT)),x=R.filter((e=>e.measurementType===l.VALUE_TYPES.POINT));return r.createElement(r.Fragment,null,r.createElement("div",{className:"invisible-scrollbar overflow-y-auto overflow-x-hidden",ref:M,"data-cy":"trackedMeasurements-panel"},D.key&&r.createElement(u.YL,{date:w(D.date),modality:D.modality,description:D.description}),r.createElement(u.wt,{title:"Measurements",data:_,servicesManager:t,onClick:C,onEdit:b}),0!==x.length&&r.createElement(u.wt,{title:"Additional Findings",data:x,servicesManager:t,onClick:C,onEdit:b})),r.createElement("div",{className:"flex justify-center p-4"},r.createElement(f,{onExportClick:async function(){const e=l.getMeasurements().filter((e=>m===e.referenceStudyUID&&g.includes(e.referenceSeriesUID)));h(e,l)},onCreateReportClick:()=>{I("SAVE_REPORT",{viewportId:a.activeViewportId,isBackupSave:!0})},disabled:0===x.length&&0===_.length})))}M.propTypes={servicesManager:i().shape({services:i().shape({measurementService:i().shape({getMeasurements:i().func.isRequired,VALUE_TYPES:i().object.isRequired}).isRequired}).isRequired}).isRequired};const A=M;const C=function(e){let{commandsManager:t,extensionManager:n,servicesManager:s}=e;return[{name:"seriesList",iconName:"tab-studies",iconLabel:"Studies",label:"Studies",component:E.bind(null,{commandsManager:t,extensionManager:n,servicesManager:s})},{name:"trackedMeasurements",iconName:"tab-linear",iconLabel:"Measure",label:"Measurements",component:A.bind(null,{commandsManager:t,extensionManager:n,servicesManager:s})}]};function b(){return b=Object.assign?Object.assign.bind():function(e){for(var t=1;t<arguments.length;t++){var n=arguments[t];for(var s in n)Object.prototype.hasOwnProperty.call(n,s)&&(e[s]=n[s])}return e},b.apply(this,arguments)}const N=r.lazy((()=>n.e(822).then(n.bind(n,86822)))),_=e=>r.createElement(r.Suspense,{fallback:r.createElement("div",null,"Loading...")},r.createElement(N,e));const x=function(e){let{servicesManager:t,commandsManager:n,extensionManager:s}=e;return[{name:"cornerstone-tracked",component:e=>r.createElement(_,b({servicesManager:t,commandsManager:n,extensionManager:s},e))}]},P={id:JSON.parse('{"u2":"@ohif/extension-measurement-tracking"}').u2,getContextModule:s.Z,getPanelModule:C,getViewportModule:x}}}]);
//# sourceMappingURL=19.bundle.97cd1d5f412be83022cf.js.map