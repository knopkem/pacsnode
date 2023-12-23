"use strict";(self.webpackChunk=self.webpackChunk||[]).push([[359],{22359:(o,e,t)=>{t.r(e),t.d(e,{default:()=>N});var n=t(71771),i=t(71783);const s={CT:"ctToolGroup",PT:"ptToolGroup",Fusion:"fusionToolGroup",MIP:"mipToolGroup",default:"default"};const l=function(o,e,t,n){!function(o,e,t,n){const i={active:[{toolName:o.WindowLevel,bindings:[{mouseButton:e.MouseBindings.Primary}]},{toolName:o.Pan,bindings:[{mouseButton:e.MouseBindings.Auxiliary}]},{toolName:o.Zoom,bindings:[{mouseButton:e.MouseBindings.Secondary}]},{toolName:o.StackScrollMouseWheel,bindings:[]}],passive:[{toolName:o.Length},{toolName:o.ArrowAnnotate,configuration:{getTextCallback:(o,e)=>{n.runCommand("arrowTextCallback",{callback:o,eventDetails:e})},changeTextCallback:(o,e,t)=>n.runCommand("arrowTextCallback",{callback:t,data:o,eventDetails:e})}},{toolName:o.Bidirectional},{toolName:o.DragProbe},{toolName:o.Probe},{toolName:o.EllipticalROI},{toolName:o.RectangleROI},{toolName:o.StackScroll},{toolName:o.Angle},{toolName:o.CobbAngle},{toolName:o.Magnify}],enabled:[{toolName:o.SegmentationDisplay}],disabled:[{toolName:o.Crosshairs,configuration:{viewportIndicators:!1,autoPan:{enabled:!1,panSize:10}}}]};t.createToolGroupAndAddTools(s.CT,i),t.createToolGroupAndAddTools(s.PT,{active:i.active,passive:[...i.passive,{toolName:"RectangleROIStartEndThreshold"}],enabled:i.enabled,disabled:i.disabled}),t.createToolGroupAndAddTools(s.Fusion,i),t.createToolGroupAndAddTools(s.default,i);const l={active:[{toolName:o.VolumeRotateMouseWheel,configuration:{rotateIncrementDegrees:.1}},{toolName:o.MipJumpToClick,configuration:{toolGroupId:s.PT},bindings:[{mouseButton:e.MouseBindings.Primary}]}],enabled:[{toolName:o.SegmentationDisplay}]};t.createToolGroupAndAddTools(s.MIP,l)}(o,e,t,n)},{windowLevelPresets:a}=n.defaults;function r(o,e,t,n,i,s){return{id:e,icon:t,label:n,type:o,commands:i,tooltip:s}}function d(o,e){return{id:o,label:o,type:"action",commands:[{commandName:"setFusionPTColormap",commandOptions:{toolGroupId:s.Fusion,colormap:e}}]}}r.bind(null,"action"),r.bind(null,"toggle");const c=r.bind(null,"tool");function u(o,e,t){return{id:o.toString(),title:e,subtitle:t,type:"action",commands:[{commandName:"setWindowLevel",commandOptions:{...a[o]},context:"CORNERSTONE"}]}}function m(o,e,t){return t.map((t=>({commandName:o,commandOptions:{toolName:e,toolGroupId:t},context:"CORNERSTONE"})))}const p=[{id:"MeasurementTools",type:"ohif.splitButton",props:{groupId:"MeasurementTools",isRadio:!0,primary:c("Length","tool-length","Length",[...m("setToolActive","Length",[s.CT,s.PT,s.Fusion])],"Length"),secondary:{icon:"chevron-down",label:"",isActive:!0,tooltip:"More Measure Tools"},items:[c("Length","tool-length","Length",[...m("setToolActive","Length",[s.CT,s.PT,s.Fusion])],"Length Tool"),c("Bidirectional","tool-bidirectional","Bidirectional",[...m("setToolActive","Bidirectional",[s.CT,s.PT,s.Fusion])],"Bidirectional Tool"),c("ArrowAnnotate","tool-annotate","Annotation",[...m("setToolActive","ArrowAnnotate",[s.CT,s.PT,s.Fusion])],"Arrow Annotate"),c("EllipticalROI","tool-elipse","Ellipse",[...m("setToolActive","EllipticalROI",[s.CT,s.PT,s.Fusion])],"Ellipse Tool")]}},{id:"Zoom",type:"ohif.radioGroup",props:{type:"tool",icon:"tool-zoom",label:"Zoom",commands:[...m("setToolActive","Zoom",[s.CT,s.PT,s.Fusion])]}},{id:"MPR",type:"ohif.action",props:{type:"toggle",icon:"icon-mpr",label:"MPR",commands:[{commandName:"toggleHangingProtocol",commandOptions:{protocolId:"mpr"},context:"DEFAULT"}]}},{id:"WindowLevel",type:"ohif.splitButton",props:{groupId:"WindowLevel",primary:c("WindowLevel","tool-window-level","Window Level",[...m("setToolActive","WindowLevel",[s.CT,s.PT,s.Fusion])],"Window Level"),secondary:{icon:"chevron-down",label:"W/L Manual",isActive:!0,tooltip:"W/L Presets"},isAction:!0,renderer:i.eJ,items:[u(1,"Soft tissue","400 / 40"),u(2,"Lung","1500 / -600"),u(3,"Liver","150 / 90"),u(4,"Bone","2500 / 480"),u(5,"Brain","80 / 40")]}},{id:"Crosshairs",type:"ohif.radioGroup",props:{type:"tool",icon:"tool-crosshair",label:"Crosshairs",commands:[...m("setToolActive","Crosshairs",[s.CT,s.PT,s.Fusion])]}},{id:"Pan",type:"ohif.radioGroup",props:{type:"tool",icon:"tool-move",label:"Pan",commands:[...m("setToolActive","Pan",[s.CT,s.PT,s.Fusion])]}},{id:"RectangleROIStartEndThreshold",type:"ohif.radioGroup",props:{type:"tool",icon:"tool-create-threshold",label:"Rectangle ROI Threshold",commands:[...m("setToolActive","RectangleROIStartEndThreshold",[s.PT]),{commandName:"displayNotification",commandOptions:{title:"RectangleROI Threshold Tip",text:"RectangleROI Threshold tool should be used on PT Axial Viewport",type:"info"}},{commandName:"setViewportActive",commandOptions:{viewportId:"ptAXIAL"}}]}},{id:"fusionPTColormap",type:"ohif.splitButton",props:{groupId:"fusionPTColormap",primary:c("fusionPTColormap","tool-fusion-color","Fusion PT Colormap",[],"Fusion PT Colormap"),secondary:{icon:"chevron-down",label:"PT Colormap",isActive:!0,tooltip:"PET Image Colormap"},isAction:!0,items:[d("HSV","hsv"),d("Hot Iron","hot_iron"),d("S PET","s_pet"),d("Red Hot","red_hot"),d("Perfusion","perfusion"),d("Rainbow","rainbow_2"),d("SUV","suv"),d("GE 256","ge_256"),d("GE","ge"),d("Siemens","siemens")]}}],T=JSON.parse('{"u2":"@ohif/mode-tmtv"}').u2;const{MetadataProvider:g}=n.classes,h="@ohif/extension-default.layoutTemplateModule.viewerLayout",v="@ohif/extension-default.sopClassHandlerModule.stack",f="@ohif/extension-default.panelModule.seriesList",y="@ohif/extension-cornerstone.viewportModule.cornerstone",C="@ohif/extension-tmtv.hangingProtocolModule.ptCT",P="@ohif/extension-tmtv.panelModule.petSUV",b="@ohif/extension-tmtv.panelModule.ROIThresholdSeg",S={"@ohif/extension-default":"^3.0.0","@ohif/extension-cornerstone":"^3.0.0","@ohif/extension-tmtv":"^3.0.0"};let I=[];const N={id:T,modeFactory:function(o){let{modeConfiguration:e}=o;return{id:T,routeName:"tmtv",displayName:"Total Metabolic Tumor Volume",onModeEnter:o=>{let{servicesManager:e,extensionManager:t,commandsManager:n}=o;const{toolbarService:i,toolGroupService:a,hangingProtocolService:r,displaySetService:d}=e.services,c=t.getModuleEntry("@ohif/extension-cornerstone.utilityModule.tools"),{toolNames:u,Enums:m}=c.exports;l(u,m,a,n);const{unsubscribe:T}=a.subscribe(a.EVENTS.VIEWPORT_ADDED,(()=>{const{displaySetMatchDetails:o}=r.getMatchDetails();!function(o,e,t,n){const i=o.get("ctDisplaySet");if(!i)return;const{SeriesInstanceUID:l}=i,a=n.getDisplaySetsForSeries(l),r={...t.getToolConfiguration(s.Fusion,e.Crosshairs),filterActorUIDsToSetSlabThickness:[a[0].displaySetInstanceUID]};t.setToolConfiguration(s.Fusion,e.Crosshairs,r)}(o,u,a,d),function(o,e,t,n){const i=o.get("ptDisplaySet");if(!i)return;const{SeriesInstanceUID:l}=i,a=n.getDisplaySetsForSeries(l);if(!a||0===a.length)return;const r=t.getToolConfiguration(s.Fusion,e.WindowLevel),d=t.getToolConfiguration(s.Fusion,e.EllipticalROI),c=`cornerstoneStreamingImageVolume:${a[0].displaySetInstanceUID}`,u={...r,volumeId:c},m={...d,volumeId:c};t.setToolConfiguration(s.Fusion,e.WindowLevel,u),t.setToolConfiguration(s.Fusion,e.EllipticalROI,m)}(o,u,a,d),i.recordInteraction({groupId:"WindowLevel",interactionType:"tool",commands:[{commandName:"setToolActive",commandOptions:{toolName:u.WindowLevel,toolGroupId:s.CT},context:"CORNERSTONE"},{commandName:"setToolActive",commandOptions:{toolName:u.WindowLevel,toolGroupId:s.PT},context:"CORNERSTONE"},{commandName:"setToolActive",commandOptions:{toolName:u.WindowLevel,toolGroupId:s.Fusion},context:"CORNERSTONE"}]})}));I.push(T),i.init(t),i.addButtons(p),i.createButtonSection("primary",["MeasurementTools","Zoom","WindowLevel","Crosshairs","Pan","RectangleROIStartEndThreshold","fusionPTColormap"]),r.addCustomAttribute("getPTVOIRange","get PT VOI based on corrected or not",(o=>{const e=o.find((o=>"PT"===o.Modality));if(!e)return;const{imageId:t}=e.images[0],n=g.get("scalingModule",t);return n&&n.suvbw?{windowWidth:5,windowCenter:2.5}:void 0}))},onModeExit:o=>{let{servicesManager:e}=o;const{toolGroupService:t,syncGroupService:n,segmentationService:i,cornerstoneViewportService:s}=e.services;I.forEach((o=>o())),t.destroy(),n.destroy(),i.destroy(),s.destroy()},validationTags:{study:[],series:[]},isValidMode:o=>{let{modalities:e,study:t}=o;const n=e.split("\\");return n.includes("CT")&&n.includes("PT")&&!["SM"].some((o=>n.includes(o)))&&"1.3.6.1.4.1.12842.1.1.14.3.20220915.105557.468.2963630849"!==t.studyInstanceUid},routes:[{path:"tmtv",layoutTemplate:o=>{let{location:e,servicesManager:t}=o;return{id:h,props:{leftPanels:[f],leftPanelDefaultClosed:!0,rightPanels:[b,P],viewports:[{namespace:y,displaySetsToDisplay:[v]}]}}}}],extensions:S,hangingProtocol:C,sopClassHandlers:[v],hotkeys:[...n.dD.defaults.hotkeyBindings],...e}},extensionDependencies:S}}}]);
//# sourceMappingURL=359.bundle.e32228f9015077353eac.js.map