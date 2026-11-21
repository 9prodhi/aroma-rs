import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
run(event) { switch (event.type) { case "save": this.save(); break; default: this.reset(); } }
}
